//! ESP-NOW wrapper (esp-idf-sys).
//!
//! We keep this thin and mostly safe:
//! - init/deinit
//! - add peer
//! - send
//! - register receive callback that forwards frames into an `mpsc::Sender`
//!
//! Transport uses **ESP-NOW v2** (up to [`MAX_FRAME_LEN`] bytes per frame on IDF 6.x).
//! IDF recommends spacing large sends on send-callback completion; wardrive bursts
//! should be soak-tested if `ESPNOW_TX_FAILED` rises.
//!
//! IMPORTANT: ESP-NOW is channel-sensitive. The control node's SoftAP stays on a fixed
//! channel (e.g. 11) and sniffers send on that channel *after* finishing a scan.
//!
//! `wifi_interface_t` for peers must match an **enabled** interface (ESP-IDF):
//! use `WIFI_IF_AP` on the control node (APSTA + SoftAP) and `WIFI_IF_STA` on sniffers.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use core::ffi::CStr;
use esp_idf_sys as sys;
use log::{info, warn};

use crate::spiram_queue::{SyncSender, TrySendError};

include!(concat!(env!("OUT_DIR"), "/espnow_pmk.rs"));

static RX_TX: Mutex<Option<SyncSender<RxFrame>>> = Mutex::new(None);

/// Cache of `(mac, channel, ifidx)` triples we've already added to the IDF peer table.
/// Avoids paying for `esp_now_mod_peer` on every periodic command/ping when N sniffers
/// are paired (see `ESPNOW_PEER_MOD_ATTEMPTS`).
static PEER_CACHE: Mutex<Option<HashSet<([u8; 6], u8, u32)>>> = Mutex::new(None);

fn peer_cache_contains(key: &([u8; 6], u8, u32)) -> bool {
    if let Ok(g) = PEER_CACHE.lock() {
        return g.as_ref().map(|s| s.contains(key)).unwrap_or(false);
    }
    false
}

fn peer_cache_insert(key: ([u8; 6], u8, u32)) {
    if let Ok(mut g) = PEER_CACHE.lock() {
        g.get_or_insert_with(HashSet::new).insert(key);
    }
}

/// After `esp_now_send` fails, drop cache entries for `mac` so the next `add_peer` runs the
/// IDF add/mod path again (peer table can diverge without `esp_now_deinit`).
fn peer_cache_remove_mac(mac: &[u8; 6]) {
    if let Ok(mut g) = PEER_CACHE.lock() {
        if let Some(s) = g.as_mut() {
            s.retain(|k| &k.0 != mac);
            if s.is_empty() {
                *g = None;
            }
        }
    }
}

pub const BROADCAST_MAC: [u8; 6] = [0xff; 6];

/// ESP-NOW v2 application payload cap (IDF `ESP_NOW_MAX_DATA_LEN_V2`). Keep in sync with
/// [`wardriver_proto::ESP_NOW_MAX_FRAME_LEN`].
pub const MAX_FRAME_LEN: usize = 1470;

/// Bounded ESP-NOW RX queue; [`spiram_queue::bounded_channel`] preallocates **one contiguous**
/// ring in SPIRAM (internal fallback) of `capacity * size_of::<RxFrame>()`.
///
/// **Sniffer / default:** [`RX_QUEUE_CAPACITY`] (128 slots). Sniffers tune via
/// [`configure_rx_queue_capacity`] from [`crate::memory_policy`].
///
/// **Control:** fixed [`ESPNOW_RX_QUEUE_CAPACITY_CONTROL`] (256 slots, ~380 KiB at v2 MTU) —
/// not heap-tuned. Requires contiguous SPIRAM at boot; panic if alloc fails.
pub const RX_FRAME_SLOT_BYTES: usize = 6 + MAX_FRAME_LEN + 4;

/// Default sniffer RX ring depth when [`configure_rx_queue_capacity`] is not called.
pub const RX_QUEUE_CAPACITY: usize = 128;

/// Fixed control-node ESP-NOW RX ring depth (wardrive fleet ingest bottleneck).
pub const ESPNOW_RX_QUEUE_CAPACITY_CONTROL: usize = 256;

/// SPIRAM reserved for the control ESP-NOW RX ring (`ESPNOW_RX_QUEUE_CAPACITY_CONTROL` slots).
pub const ESPNOW_RX_QUEUE_RING_BYTES_CONTROL: usize =
    ESPNOW_RX_QUEUE_CAPACITY_CONTROL * RX_FRAME_SLOT_BYTES;
static RX_QUEUE_CAPACITY_RUNTIME: OnceLock<usize> = OnceLock::new();

pub fn configure_rx_queue_capacity(cap: usize) {
    let _ = RX_QUEUE_CAPACITY_RUNTIME.set(cap.max(16));
}

pub fn rx_queue_capacity() -> usize {
    RX_QUEUE_CAPACITY_RUNTIME
        .get()
        .copied()
        .unwrap_or(RX_QUEUE_CAPACITY)
}

#[derive(Clone, Debug)]
pub struct RxFrame {
    pub src: [u8; 6],
    data: [u8; MAX_FRAME_LEN],
    len: usize,
}

impl RxFrame {
    pub fn payload(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// Apply compile-time PMK after `esp_now_init` (and after `reinit_stack` re-inits).
fn apply_espnow_pmk() -> Result<()> {
    unsafe {
        let rc = sys::esp_now_set_pmk(ESPNOW_PMK_BYTES.as_ptr() as *mut u8);
        if rc != 0 {
            return Err(anyhow!("esp_now_set_pmk rc={rc}"));
        }
    }
    info!("espnow: PMK configured (encrypted unicast peers)");
    Ok(())
}

pub fn init(rx_tx: SyncSender<RxFrame>) -> Result<()> {
    // WiFi must already be initialized and started by esp-idf-svc.
    {
        let g = RX_TX
            .lock()
            .map_err(|_| anyhow!("espnow rx mutex poisoned"))?;
        if g.is_some() {
            return Err(anyhow!("espnow init called while already initialized"));
        }
    }
    unsafe {
        let rc = sys::esp_now_init();
        if rc != 0 {
            return Err(anyhow!("esp_now_init rc={rc}"));
        }
        apply_espnow_pmk()?;
        let mut g = RX_TX
            .lock()
            .map_err(|_| anyhow!("espnow rx mutex poisoned"))?;
        *g = Some(rx_tx);
        let rc2 = sys::esp_now_register_recv_cb(Some(recv_cb));
        if rc2 != 0 {
            *g = None;
            let _ = sys::esp_now_deinit();
            return Err(anyhow!("esp_now_register_recv_cb rc={rc2}"));
        }
    }
    log_espnow_version()?;
    info!("espnow: initialized (max_frame_len={MAX_FRAME_LEN})");
    Ok(())
}

/// Log IDF ESP-NOW stack version; fail boot if v2 is unavailable (required for fleet MTU).
fn log_espnow_version() -> Result<()> {
    let mut version: u32 = 0;
    let rc = unsafe { sys::esp_now_get_version(&mut version as *mut u32) };
    if rc != 0 {
        return Err(anyhow!("esp_now_get_version rc={rc}"));
    }
    info!("espnow: stack version={version} (2 = v2 / 1470 B MTU)");
    if version < 2 {
        return Err(anyhow!(
            "espnow: IDF reports version {version}; v2 required for MAX_FRAME_LEN={MAX_FRAME_LEN}"
        ));
    }
    Ok(())
}

/// True when the recv callback has a live SPIRAM queue sender (post-[`init`]).
pub fn rx_sender_wired() -> bool {
    RX_TX
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|_| ()))
        .is_some()
}

fn peer_cache_clear() {
    if let Ok(mut g) = PEER_CACHE.lock() {
        *g = None;
    }
}

fn reinit_stack_inner() -> Result<()> {
    unsafe {
        let rc = sys::esp_now_deinit();
        if rc != 0 {
            warn!("espnow: esp_now_deinit (reinit) rc={rc}");
        }
    }
    peer_cache_clear();
    unsafe {
        let rc = sys::esp_now_init();
        if rc != 0 {
            return Err(anyhow!("esp_now_init (reinit) rc={rc}"));
        }
        apply_espnow_pmk()?;
        let rc2 = sys::esp_now_register_recv_cb(Some(recv_cb));
        if rc2 != 0 {
            let _ = sys::esp_now_deinit();
            return Err(anyhow!("esp_now_register_recv_cb (reinit) rc={rc2}"));
        }
    }
    info!("espnow: stack reinitialized");
    Ok(())
}

/// Tear down ESP-NOW before stopping the WiFi driver (e.g. BLE capture window).
/// Clears peer cache; keeps [`RX_TX`] wired for a later [`reinit_stack`].
#[cfg(feature = "sniffer")]
pub fn deinit_stack() -> Result<()> {
    unsafe {
        let rc = sys::esp_now_deinit();
        if rc != 0 {
            warn!("espnow: esp_now_deinit rc={rc}");
        }
    }
    peer_cache_clear();
    Ok(())
}

/// De-init and re-init ESP-NOW + recv callback; clears peer cache. RX [`SyncSender`] in
/// [`RX_TX`] is kept so the sniffer main loop does not need a second sender clone.
#[cfg(feature = "sniffer")]
pub fn reinit_stack() -> Result<()> {
    reinit_stack_inner()
}

/// Control node: after STA / SoftAP WiFi reconfiguration, re-bind ESP-NOW to the driver and
/// re-add the broadcast peer (sniffer discovery + periodic `Ping` use it).
#[cfg(feature = "control")]
pub fn refresh_stack_after_wifi(
    broadcast_ch: u8,
    broadcast_if: sys::wifi_interface_t,
) -> Result<()> {
    reinit_stack_inner()?;
    add_broadcast_peer(broadcast_ch, broadcast_if)?;
    info!(
        "espnow: control stack refresh complete (broadcast ch={} ifidx={})",
        broadcast_ch, broadcast_if as u32
    );
    Ok(())
}

fn apply_peer_rate_config(mac: &[u8; 6]) {
    // HE20 MCS6 SGI: fixed fleet ESP-NOW rate after peer table add/mod.
    unsafe {
        let mut config: sys::esp_now_rate_config_t = core::mem::zeroed();
        config.phymode = sys::wifi_phy_mode_t_WIFI_PHY_MODE_HE20;
        config.rate = sys::wifi_phy_rate_t_WIFI_PHY_RATE_MCS6_SGI;
        config.ersu = false;
        config.dcm = false;
        let rc = sys::esp_now_set_peer_rate_config(mac.as_ptr(), &mut config);
        if rc != 0 {
            warn!(
                "espnow: esp_now_set_peer_rate_config mac={:02x}:{:02x}:… rc={rc}",
                mac[0], mac[1]
            );
        }
    }
}

pub fn add_peer(mac: [u8; 6], channel: u8, ifidx: sys::wifi_interface_t) -> Result<()> {
    let cache_key = (mac, channel, ifidx as u32);
    // Fast path: we already added this exact `(mac, channel, ifidx)` triple. Skip both
    // `esp_now_add_peer` and the `mod_peer` fallback to keep periodic fan-outs cheap on
    // the control node when many sniffers are paired.
    if peer_cache_contains(&cache_key) {
        return Ok(());
    }
    unsafe {
        let mut peer: sys::esp_now_peer_info_t = core::mem::zeroed();
        peer.peer_addr = mac;
        peer.ifidx = ifidx;
        peer.channel = channel;
        // Encrypted unicast; broadcast discovery stays unencrypted (IDF limitation / interop).
        peer.encrypt = mac != BROADCAST_MAC;
        let rc = sys::esp_now_add_peer(&peer);
        if rc == sys::ESP_ERR_ESPNOW_EXIST as i32 {
            wardriver_stats::ESPNOW_PEER_MOD_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            // IMPORTANT: If a peer already exists but was created on a different channel/ifidx,
            // ESP-IDF will refuse to send and log:
            //   "Peer channel is not equal to the home channel, send fail!"
            //
            // Fix up the peer entry to match our desired channel. This is critical for the
            // control node's broadcast peer (`ff:ff:ff:ff:ff:ff`) and for sniffers that were
            // previously paired on a different channel.
            let rc_mod = sys::esp_now_mod_peer(&peer);
            if rc_mod != 0 {
                wardriver_stats::ESPNOW_PEER_MOD_FAILED.fetch_add(1, Ordering::Relaxed);
                warn!(
                    "espnow: esp_now_mod_peer mac={:02x}:{:02x}:… ch={} ifidx={} rc={}",
                    mac[0], mac[1], channel, ifidx, rc_mod
                );
                return Err(anyhow!("esp_now_mod_peer rc={rc_mod}"));
            }
            info!(
                "espnow: peer existed; updated mac={:02x}:{:02x}:… ch={} ifidx={}",
                mac[0], mac[1], channel, ifidx
            );
            apply_peer_rate_config(&mac);
            peer_cache_insert(cache_key);
            return Ok(());
        }

        if rc != 0 {
            warn!(
                "espnow: esp_now_add_peer mac={:02x}:{:02x}:… ch={} ifidx={} rc={}",
                mac[0], mac[1], channel, ifidx, rc
            );
            return Err(anyhow!("esp_now_add_peer rc={rc}"));
        }
        log::debug!(
            "espnow: add_peer ok mac={:02x}:{:02x}:… ch={} ifidx={} rc={}",
            mac[0],
            mac[1],
            channel,
            ifidx,
            rc
        );
    }
    apply_peer_rate_config(&mac);
    peer_cache_insert(cache_key);
    Ok(())
}

/// Remove a peer from the IDF ESP-NOW table and evict all `PEER_CACHE` entries for `mac`.
/// Frees an encrypted-peer slot for wolfPACK rotation after post-dump grace.
pub fn del_peer(mac: [u8; 6]) -> Result<()> {
    unsafe {
        let rc = sys::esp_now_del_peer(mac.as_ptr());
        if rc != 0 {
            warn!(
                "espnow: esp_now_del_peer mac={:02x}:{:02x}:… rc={}",
                mac[0], mac[1], rc
            );
            return Err(anyhow!("esp_now_del_peer rc={rc}"));
        }
    }
    peer_cache_remove_mac(&mac);
    Ok(())
}

/// Block ESP-NOW TX while tuned to a US DFS primary (sniffer RX-only policy).
#[cfg(feature = "sniffer")]
fn sniffer_dfs_espnow_send_guard() -> Result<(), &'static str> {
    let mut primary: u8 = 0;
    let mut second = sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE;
    let rc = unsafe { sys::esp_wifi_get_channel(&mut primary, &mut second) };
    if rc != 0 || primary == 0 {
        return Ok(());
    }
    if matches!(
        primary,
        52 | 56 | 60 | 64 | 100 | 104 | 108 | 112 | 116 | 120 | 124 | 128 | 132 | 136 | 140 | 144
    ) {
        wardriver_stats::increment_dfs_espnow_blocked();
        Err("dfs_channel")
    } else {
        Ok(())
    }
}

pub fn send(dst: [u8; 6], data: &[u8]) -> Result<()> {
    if data.len() > MAX_FRAME_LEN {
        return Err(anyhow!(
            "espnow send len={} exceeds MAX_FRAME_LEN={MAX_FRAME_LEN}",
            data.len()
        ));
    }
    #[cfg(feature = "sniffer")]
    if let Err(e) = sniffer_dfs_espnow_send_guard() {
        return Err(anyhow!("espnow send blocked on DFS primary ({e})"));
    }
    unsafe {
        let rc = sys::esp_now_send(dst.as_ptr(), data.as_ptr(), data.len());
        if rc != 0 {
            wardriver_stats::ESPNOW_TX_FAILED.fetch_add(1, Ordering::Relaxed);
            peer_cache_remove_mac(&dst);
            let name = CStr::from_ptr(sys::esp_err_to_name(rc)).to_string_lossy();
            warn!(
                "espnow: esp_now_send dst={:02x}:{:02x}:… len={} rc={} ({})",
                dst[0],
                dst[1],
                data.len(),
                rc,
                name
            );
            return Err(anyhow!("esp_now_send rc={rc}"));
        }
    }
    Ok(())
}

pub fn add_broadcast_peer(channel: u8, ifidx: sys::wifi_interface_t) -> Result<()> {
    add_peer(BROADCAST_MAC, channel, ifidx)
}

/// Set the WiFi primary channel (secondary is NONE).
///
/// Needed because ESP-NOW TX/RX happens on the current WiFi channel.
#[cfg(any(feature = "sniffer", feature = "control"))]
pub fn set_channel(primary: u8) -> Result<()> {
    unsafe {
        let rc = sys::esp_wifi_set_channel(primary, sys::wifi_second_chan_t_WIFI_SECOND_CHAN_NONE);
        if rc != 0 {
            let name = CStr::from_ptr(sys::esp_err_to_name(rc)).to_string_lossy();
            return Err(anyhow!(
                "esp_wifi_set_channel ch={primary} rc={rc} ({name})"
            ));
        }
    }
    Ok(())
}

unsafe extern "C" fn recv_cb(
    esp_now_info: *const sys::esp_now_recv_info_t,
    data: *const u8,
    len: core::ffi::c_int,
) {
    if esp_now_info.is_null() || data.is_null() || len <= 0 {
        return;
    }
    wardriver_stats::ESPNOW_RX_CB_TOTAL.fetch_add(1, Ordering::Relaxed);
    let len = len as usize;
    if len > MAX_FRAME_LEN {
        wardriver_stats::ESPNOW_RX_DROPPED_OVERSIZED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let info = unsafe { &*esp_now_info };
    if info.src_addr.is_null() {
        return;
    }
    let mut src = [0u8; 6];
    unsafe {
        core::ptr::copy_nonoverlapping(info.src_addr, src.as_mut_ptr(), 6);
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len) };
    let mut frame_data = [0u8; MAX_FRAME_LEN];
    frame_data[..len].copy_from_slice(slice);
    let frame = RxFrame {
        src,
        data: frame_data,
        len,
    };
    let send_result = match RX_TX.try_lock() {
        Ok(g) => g.as_ref().map(|tx| tx.try_send(frame)),
        Err(_) => {
            wardriver_stats::ESPNOW_RX_DROPPED_MUTEX_BUSY.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    match send_result {
        Some(Ok(())) => {
            wardriver_stats::queue_inflight_inc(
                &wardriver_stats::ESPNOW_RX_QUEUE_INFLIGHT,
                &wardriver_stats::ESPNOW_RX_QUEUE_HWM,
            );
        }
        Some(Err(TrySendError::Full(_))) => {
            wardriver_stats::ESPNOW_RX_DROPPED_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
        }
        Some(Err(TrySendError::Disconnected(_))) => {
            wardriver_stats::ESPNOW_RX_DROPPED_DISCONNECTED.fetch_add(1, Ordering::Relaxed);
        }
        None => {
            wardriver_stats::ESPNOW_RX_CB_NO_SENDER.fetch_add(1, Ordering::Relaxed);
        }
    }
}
