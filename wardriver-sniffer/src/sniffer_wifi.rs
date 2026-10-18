//! WiFi boot, regulatory, channel-recovery, and recon DFS TX queue helpers for the sniffer role.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use esp_idf_svc::wifi::{BlockingWifi, Configuration, EspWifi};
use log::{info, warn};

use wardriver_shared::espnow;

// Recon may tune DFS primaries; ESP-NOW TX is blocked there, so buffer probe frags and
// flush after `recon_flush_dfs_tx_queue` returns to the control channel.
const RECON_DFS_TX_QUEUE_CAP: usize = 48;
static RECON_DFS_TX_QUEUE: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

pub(crate) fn recon_push_dfs_tx_payload(payload: Vec<u8>) {
    let Ok(mut q) = RECON_DFS_TX_QUEUE.lock() else {
        return;
    };
    while q.len() >= RECON_DFS_TX_QUEUE_CAP {
        q.remove(0);
    }
    q.push(payload);
}

pub(crate) fn recon_flush_dfs_tx_queue(control_ch: u8, mac: [u8; 6]) {
    let _ = espnow::set_channel(control_ch);
    let mut q = RECON_DFS_TX_QUEUE.lock().ok();
    let Some(queue) = q.as_mut() else {
        return;
    };
    for payload in queue.drain(..) {
        let _ = espnow::send(mac, &payload);
    }
}

/// Centralized WiFi regulatory setup for the sniffer role.
pub fn configure_wifi_regulatory() {
    let _rc = wardriver_shared::wifi_regulatory::configure_wifi_country_us();
    wardriver_shared::wifi_regulatory::set_sniffer_band_mode_auto();
}

/// Probe non-DFS 5 GHz capability (ch 36 tune OK).
pub fn probe_five_ghz_capable() -> u8 {
    const FIVE_GHZ_CH: u8 = 36;
    u8::from(espnow::set_channel(FIVE_GHZ_CH).is_ok())
}

static SNIFFER_FIVE_GHZ_CAPABLE: OnceLock<u8> = OnceLock::new();

pub(crate) fn set_sniffer_five_ghz_capable(cap: u8) {
    let _ = SNIFFER_FIVE_GHZ_CAPABLE.set(cap);
}

pub(crate) fn sniffer_five_ghz_capable() -> u8 {
    SNIFFER_FIVE_GHZ_CAPABLE.get().copied().unwrap_or(0)
}

/// Fixed for this boot so the control node can drop stale per-sniffer time-sync offsets
/// when this sniffer reboots ([`proto::SnifferAckV1::sniffer_boot_id`]).
static SNIFFER_BOOT_ID: OnceLock<u32> = OnceLock::new();

pub(crate) fn sniffer_boot_id() -> u32 {
    *SNIFFER_BOOT_ID.get_or_init(|| {
        let mut x = unsafe { esp_idf_sys::esp_random() };
        if x == 0 {
            x = 1;
        }
        x
    })
}

const SET_CHANNEL_RETRY_ATTEMPTS: u8 = 4;
const SET_CHANNEL_RETRY_SLEEP_MS: u64 = 20;

const SNIFFER_ESPNOW_IF: esp_idf_sys::wifi_interface_t = esp_idf_sys::wifi_interface_t_WIFI_IF_STA;

/// Re-register ESP-NOW peers after `deinit_stack` / `reinit_stack` (boot parity).
pub(crate) fn restore_sniffer_espnow_peers(control_ch: u8, control_mac: Option<[u8; 6]>) -> bool {
    let ch = control_ch.max(1);
    if espnow::add_broadcast_peer(ch, SNIFFER_ESPNOW_IF).is_err() {
        warn!("sniffer: restore espnow peers: broadcast add failed (ch={ch})");
        return false;
    }
    if let Some(mac) = control_mac {
        if let Err(e) = espnow::add_peer(mac, ch, SNIFFER_ESPNOW_IF) {
            warn!("sniffer: restore espnow peers: control add_peer failed: {e:#}");
            return false;
        }
    }
    true
}

/// Best-effort: deinit ESP-NOW and stop the WiFi driver before a BLE capture window.
/// Returns `false` if pause failed (caller may run capture in degraded coexistence mode).
pub(crate) fn pause_wifi_for_ble_capture(wifi: &mut BlockingWifi<EspWifi<'static>>) -> bool {
    if wardriver_shared::espnow::deinit_stack().is_err() {
        warn!("sniffer: ble capture: espnow deinit failed; wifi pause skipped");
        return false;
    }
    match wifi.stop() {
        Ok(()) => {
            info!("sniffer: wifi paused for ble capture");
            true
        }
        Err(e) => {
            warn!("sniffer: ble capture: wifi.stop failed: {e:#}");
            let _ = wardriver_shared::espnow::reinit_stack();
            false
        }
    }
}

/// STA restart, ESP-NOW reinit, tune to control channel, and restore peer table.
pub(crate) fn resume_wifi_after_ble_capture(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    control_ch: u8,
    control_mac: Option<[u8; 6]>,
) -> bool {
    if wifi
        .set_configuration(&Configuration::Client(Default::default()))
        .is_err()
    {
        warn!("sniffer: ble capture resume: set_configuration failed");
        return false;
    }
    if wifi.start().is_err() {
        warn!("sniffer: ble capture resume: wifi.start failed");
        return false;
    }
    if wardriver_shared::espnow::reinit_stack().is_err() {
        warn!("sniffer: ble capture resume: espnow reinit failed");
        return false;
    }
    if !try_set_channel_with_recovery(wifi, control_ch, "ble capture resume") {
        warn!("sniffer: ble capture resume: set_channel({control_ch}) failed");
        return false;
    }
    if !restore_sniffer_espnow_peers(control_ch, control_mac) {
        warn!("sniffer: ble capture resume: espnow peer restore failed");
        return false;
    }
    info!("sniffer: wifi resumed after ble capture (ch={control_ch})");
    true
}

pub(crate) fn try_set_channel_with_recovery(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ch: u8,
    what: &str,
) -> bool {
    for attempt in 0..SET_CHANNEL_RETRY_ATTEMPTS {
        if espnow::set_channel(ch).is_ok() {
            return true;
        }
        warn!(
            "sniffer: set_channel({ch}) failed ({what}); attempt {}/{}",
            attempt.saturating_add(1),
            SET_CHANNEL_RETRY_ATTEMPTS
        );
        std::thread::sleep(Duration::from_millis(SET_CHANNEL_RETRY_SLEEP_MS));
    }

    // Bounded recovery: re-assert a minimal STA-only configuration (unassociated), then re-init
    // ESP-NOW and try again. This avoids unwinding `run_sniffer()` (which would exit `app_main()`).
    let _ = restore_sniffer_sta_unassociated(wifi);
    let _ = espnow::reinit_stack();
    espnow::set_channel(ch).is_ok()
}

/// Re-assert unassociated STA after a failed channel tune (no HTTP-dump session teardown).
pub(crate) fn restore_sniffer_sta_unassociated(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
) -> anyhow::Result<()> {
    let _ = wifi.disconnect();
    wifi.set_configuration(&Configuration::Client(Default::default()))?;
    wifi.start()?;
    Ok(())
}
