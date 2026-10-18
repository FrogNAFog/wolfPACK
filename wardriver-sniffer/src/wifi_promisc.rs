//! ESP-IDF WiFi promiscuous capture for sniffer builds.
//!
//! Registers a minimal RX callback that copies raw frames into a bounded queue; the main
//! thread parses only AP (beacon / probe response) management frames.
//!
//! **Filter:** the wardriving path uses a **management-only** promisc filter so the WiFi stack does
//! not deliver DATA/CTRL/MISC copies we would drop in software (`ingest_promisc_record` only handles
//! `WIFI_PKT_MGMT`). Beacons, probe requests, and probe responses are all management frames.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use wardriver_core::ssid_watch::SsidWatch;

use anyhow::{anyhow, Result};
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys as sys;
use wardriver_core::ieee80211_parse::{
    roaming_consortium_ie_payload_from_mgmt_mpdu, try_ap_from_mgmt_mpdu,
    try_probe_req_detail_from_mgmt_mpdu,
};
use wardriver_core::lpr_camera_wifi::{
    CameraAlertRecord, CameraProbeSample, LprCameraWifiDetector,
};
use wardriver_core::remote_id::{is_remote_id_wifi_channel, RemoteIdDetector};
use wardriver_proto::{
    ApEntryV1, DetectionAlertV1, RemoteIdAlertV1, ScanEntryExtras, SsidWatchAlertV1,
};
use wardriver_shared::spiram_queue::{
    Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};

/// Max bytes copied per frame (802.11 MSDU + header, typical beacons << this).
pub const PROMISC_MAX_COPY: usize = 1024;
/// Bounded promisc RX queue depth (fallback when runtime policy is unset).
pub const PROMISC_QUEUE_CAPACITY: usize = 48;
static PROMISC_QUEUE_CAPACITY_RUNTIME: OnceLock<usize> = OnceLock::new();

pub fn configure_promisc_queue_capacity(cap: usize) {
    let _ = PROMISC_QUEUE_CAPACITY_RUNTIME.set(cap.clamp(16, 1024));
}

pub fn promisc_queue_capacity() -> usize {
    PROMISC_QUEUE_CAPACITY_RUNTIME
        .get()
        .copied()
        .unwrap_or(PROMISC_QUEUE_CAPACITY)
}

static PROMISC_TX: Mutex<Option<SyncSender<PromiscRecord>>> = Mutex::new(None);

#[derive(Clone, Copy)]
pub struct PromiscRecord {
    pub pkt_type: sys::wifi_promiscuous_pkt_type_t,
    pub len: u16,
    pub rssi: i8,
    pub channel: u8,
    pub data: [u8; PROMISC_MAX_COPY],
}

/// Install the global queue sender and RX callback; call once from `run_sniffer` after WiFi start.
pub fn set_sender(tx: SyncSender<PromiscRecord>) -> Result<()> {
    let mut g = PROMISC_TX
        .lock()
        .map_err(|_| anyhow!("promisc tx mutex poisoned"))?;
    if g.is_some() {
        return Err(anyhow!("wifi_promisc::set_sender already called"));
    }
    *g = Some(tx);
    let rc = unsafe { sys::esp_wifi_set_promiscuous_rx_cb(Some(promisc_rx_cb)) };
    if rc != 0 {
        *g = None;
        return Err(anyhow!("esp_wifi_set_promiscuous_rx_cb rc={rc}"));
    }
    Ok(())
}

unsafe extern "C" fn promisc_rx_cb(
    buf: *mut core::ffi::c_void,
    type_: sys::wifi_promiscuous_pkt_type_t,
) {
    if buf.is_null() {
        return;
    }
    // IMPORTANT: this runs in the WiFi RX callback context; avoid blocking locks or logging here.
    let Ok(guard) = PROMISC_TX.try_lock() else {
        return;
    };
    let Some(tx) = guard.as_ref() else {
        return;
    };

    let pkt = unsafe { &*(buf as *const sys::wifi_promiscuous_pkt_t) };
    let sig = pkt.rx_ctrl.sig_len() as usize;
    if sig == 0 || sig > PROMISC_MAX_COPY {
        return;
    }

    let mut rec = PromiscRecord {
        pkt_type: type_,
        len: sig as u16,
        rssi: pkt.rx_ctrl.rssi() as i8,
        channel: pkt.rx_ctrl.channel().min(255) as u8,
        data: [0u8; PROMISC_MAX_COPY],
    };

    let payload_ptr = core::ptr::addr_of!(pkt.payload) as *const u8;
    unsafe {
        core::ptr::copy_nonoverlapping(payload_ptr, rec.data.as_mut_ptr(), sig);
    }

    match tx.try_send(rec) {
        Ok(()) => {
            wardriver_stats::queue_inflight_inc(
                &wardriver_stats::PROMISC_RX_QUEUE_INFLIGHT,
                &wardriver_stats::PROMISC_RX_QUEUE_HWM,
            );
        }
        Err(TrySendError::Full(_)) => {
            wardriver_stats::PROMISC_RX_DROPPED_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {
            // Nothing to do: sender is gone (shutdown); avoid logging from callback context.
        }
    }
}

fn set_mgmt_only_filters() -> Result<()> {
    let mask = sys::WIFI_PROMIS_FILTER_MASK_MGMT;
    let filter = sys::wifi_promiscuous_filter_t { filter_mask: mask };
    let rc = unsafe { sys::esp_wifi_set_promiscuous_filter(&filter) };
    if rc != 0 {
        return Err(anyhow!("esp_wifi_set_promiscuous_filter (mgmt) rc={rc}"));
    }
    // No control-frame subtype pass-through; main type filter is MGMT only.
    let cfilter = sys::wifi_promiscuous_filter_t { filter_mask: 0 };
    let rc2 = unsafe { sys::esp_wifi_set_promiscuous_ctrl_filter(&cfilter) };
    if rc2 != 0 {
        return Err(anyhow!("esp_wifi_set_promiscuous_ctrl_filter rc={rc2}"));
    }
    Ok(())
}

/// Max blocking wait when the queue is empty (replaces fixed 1 ms polling).
///
/// If [`wardriver_stats::PROMISC_RX_DROPPED_QUEUE_FULL`] rises under load, consider lowering this cap
/// or adding adaptive waits (shorter sleep when the queue is nearly full) — see plan follow-up.
const DWELL_RECV_TIMEOUT_CAP_MS: u64 = 15;
/// Yield after this many records in one burst (dense RF fairness vs IDLE).
const DWELL_INGEST_BATCH_YIELD: u32 = 48;

/// Timed dwell: drain bursts, then block up to [`DWELL_RECV_TIMEOUT_CAP_MS`] until deadline.
/// When `break_if` returns true after a burst, exit immediately (skip post-deadline drain).
fn promisc_dwell_loop(
    rx: &Receiver<PromiscRecord>,
    deadline: Instant,
    mut break_if: impl FnMut() -> bool,
    mut on_rec: impl FnMut(PromiscRecord),
) {
    let mut early_break = false;
    while Instant::now() < deadline {
        let mut batch: u32 = 0;
        while let Ok(rec) = rx.try_recv() {
            on_rec(rec);
            batch = batch.saturating_add(1);
            if batch >= DWELL_INGEST_BATCH_YIELD {
                batch = 0;
                std::thread::yield_now();
            }
        }
        if break_if() {
            early_break = true;
            break;
        }
        wardriver_shared::task_wdt::reset_if_subscribed();
        if Instant::now() >= deadline {
            break;
        }
        let rem = deadline.saturating_duration_since(Instant::now());
        if rem.is_zero() {
            break;
        }
        let wait = rem.min(Duration::from_millis(DWELL_RECV_TIMEOUT_CAP_MS));
        match rx.recv_timeout(wait) {
            Ok(rec) => on_rec(rec),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if break_if() {
            early_break = true;
            break;
        }
    }
    if early_break {
        return;
    }
    while let Ok(rec) = rx.try_recv() {
        on_rec(rec);
    }
}

fn drain_queue(rx: &Receiver<PromiscRecord>) {
    loop {
        match rx.try_recv() {
            Ok(_) => {
                wardriver_stats::queue_inflight_dec(&wardriver_stats::PROMISC_RX_QUEUE_INFLIGHT);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProbeReqBurstLocal {
    pub src: [u8; 6],
    pub wildcard_count: u16,
    /// RF center channel for this dwell-local burst.
    pub channel: u8,
    /// Monotonic ms32 (sniffer clock) relative to boot.
    pub first_seen_ms32: u32,
    pub last_seen_ms32: u32,
    pub rssi_min: i8,
    pub rssi_max: i8,
    /// Average RSSI in Q8 fixed point (signed dBm << 8).
    pub rssi_avg_q8: i16,
    /// `true` if any probe in this burst matched the primary LPR-camera IE TLV sequence
    /// ([`wardriver_core::lpr_camera_probe_ie::probe_req_mpdu_matches_lpr_camera_primary`]). Forwarded to
    /// control as [`wardriver_proto::ProbeReqBurstV1::primary_sig_matched`] (LPR-camera WiFi methods 2/3).
    pub primary_sig_matched: bool,
}

#[derive(Debug, Clone)]
struct ProbeReqBurstAccum {
    channel: u8,
    wildcard_count: u16,
    first_seen_ms32: u32,
    last_seen_ms32: u32,
    rssi_min: i8,
    rssi_max: i8,
    rssi_sum: i32,
    primary_sig_matched: bool,
}

impl ProbeReqBurstAccum {
    fn new(first_seen_ms32: u32, ch: u8, rssi: i8, ie_sig_match: bool) -> Self {
        Self {
            channel: ch,
            wildcard_count: 1,
            first_seen_ms32,
            last_seen_ms32: first_seen_ms32,
            rssi_min: rssi,
            rssi_max: rssi,
            rssi_sum: rssi as i32,
            primary_sig_matched: ie_sig_match,
        }
    }

    fn add(&mut self, t_ms32: u32, rssi: i8, ie_sig_match: bool) {
        self.wildcard_count = self.wildcard_count.saturating_add(1);
        self.last_seen_ms32 = t_ms32;
        if rssi < self.rssi_min {
            self.rssi_min = rssi;
        }
        if rssi > self.rssi_max {
            self.rssi_max = rssi;
        }
        self.rssi_sum = self.rssi_sum.saturating_add(rssi as i32);
        if ie_sig_match {
            self.primary_sig_matched = true;
        }
    }

    fn to_sample(&self, src: [u8; 6]) -> CameraProbeSample {
        let rssi_avg_q8 =
            wardriver_core::probe_burst::rssi_avg_q8(self.rssi_sum, self.wildcard_count);
        CameraProbeSample {
            src,
            wildcard_count: self.wildcard_count.min(u16::from(u8::MAX)) as u8,
            channel: self.channel,
            rssi_min: self.rssi_min,
            rssi_max: self.rssi_max,
            rssi_avg_q8,
            primary_sig_matched: self.primary_sig_matched,
        }
    }

    fn into_burst(self, src: [u8; 6]) -> ProbeReqBurstLocal {
        let rssi_avg_q8 =
            wardriver_core::probe_burst::rssi_avg_q8(self.rssi_sum, self.wildcard_count);
        ProbeReqBurstLocal {
            src,
            wildcard_count: self.wildcard_count,
            channel: self.channel,
            first_seen_ms32: self.first_seen_ms32,
            last_seen_ms32: self.last_seen_ms32,
            rssi_min: self.rssi_min,
            rssi_max: self.rssi_max,
            rssi_avg_q8,
            primary_sig_matched: self.primary_sig_matched,
        }
    }
}

/// One non-wildcard probe request observation (collected on sniffer, enriched on phone).
#[derive(Clone, Copy, Debug)]
struct ProbeReqWireEntry {
    src: [u8; 6],
    ssid_len: u8,
    ssid: [u8; 32],
    channel: u8,
    rssi: i8,
}

const PROBE_DEDUP_MAX_ENTRIES: usize = 512;
const PROBE_LOG_MAX_PER_ENTRY: u8 = 10;

struct ProbeLogEntry {
    last_seen_ms: u64,
    log_count: u8,
}

fn promisc_anchor_unix_ms() -> u64 {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, core::ptr::null_mut());
    }
    if tv.tv_sec <= 0 {
        return 0;
    }
    let ms = (tv.tv_sec as i64)
        .saturating_mul(1000)
        .saturating_add((tv.tv_usec as i64) / 1000);
    if ms <= 0 {
        return 0;
    }
    ms as u64
}

fn pick_wifi_alert(
    outcome: &wardriver_core::lpr_camera_wifi::CameraProbeIngestOutcome,
) -> Option<CameraAlertRecord> {
    outcome.method3.or(outcome.method2).or(outcome.method1)
}

fn remote_id_alert_to_v1(
    a: &wardriver_core::remote_id::RemoteIdAlert,
    seq: u32,
    sniffer_boot_id: u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
) -> RemoteIdAlertV1 {
    crate::remote_id_alert_wire::remote_id_alert_to_v1(
        a,
        seq,
        sniffer_boot_id,
        scan_end_ms,
        scan_end_unix_ms,
    )
}

fn wifi_alert_to_detection_alert_v1(
    a: &CameraAlertRecord,
    seq: u32,
    sniffer_boot_id: u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
    first_seen_ms32: u32,
    last_seen_ms32: u32,
    primary_sig_matched: bool,
) -> DetectionAlertV1 {
    DetectionAlertV1 {
        sniffer_id: a.sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms: scan_end_ms,
        sniffer_scan_end_unix_ms: scan_end_unix_ms,
        src: a.src,
        detection_method: a.wifi_method.as_u8(),
        channel: a.channel,
        rssi_min: a.rssi_min,
        rssi_max: a.rssi_max,
        rssi_avg_q8: a.rssi_avg_q8,
        wildcard_count: a.wildcard_count.min(u16::from(u8::MAX)) as u8,
        first_seen_ms32,
        last_seen_ms32,
        primary_sig_matched: u8::from(primary_sig_matched),
    }
}

fn try_queue_ms_oui_alert(
    camera: &mut LprCameraWifiDetector,
    mac: [u8; 6],
    channel: u8,
    rssi: i8,
    sniffer_id: u32,
    detection_disable_wifi_mask: u32,
    wifi_needs_ms_oui: bool,
    // When false (AP BSSID path), method 5 (i-PRO) is skipped; Axon method 4 still runs.
    allow_ipro: bool,
    alert_seq: u32,
    sniffer_boot_id: u32,
    alert_pending: &mut Vec<DetectionAlertV1>,
    alerted_probe_srcs: &mut HashSet<[u8; 6]>,
) {
    if !wifi_needs_ms_oui {
        return;
    }
    let now_ms = wardriver_stats::now_ms();
    let Some(a) = camera.eval_mac(
        sniffer_id,
        now_ms,
        mac,
        channel,
        rssi,
        detection_disable_wifi_mask,
        allow_ipro,
    ) else {
        return;
    };
    alerted_probe_srcs.insert(mac);
    let scan_end_ms = now_ms as u32;
    let scan_end_unix_ms = promisc_anchor_unix_ms();
    alert_pending.push(wifi_alert_to_detection_alert_v1(
        &a,
        alert_seq,
        sniffer_boot_id,
        scan_end_ms,
        scan_end_unix_ms,
        scan_end_ms,
        scan_end_ms,
        false,
    ));
}

/// Match control-side WiGLE SSID normalization (declared len cap, NUL trim).
fn ap_ssid_for_filter(ap: &ApEntryV1) -> String {
    let n = (ap.ssid_len as usize).min(32).min(ap.ssid.len());
    let slice = &ap.ssid[..n];
    let slice = slice.split(|&b| b == 0).next().unwrap_or(&[]);
    String::from_utf8_lossy(slice).into_owned()
}

/// Per-BSSID cooldown between SSID watch ESP-NOW alerts (matches control buzzer cooldown).
const SSID_WATCH_COOLDOWN_MS: u64 = 30_000;

fn try_queue_ssid_watch_beacon(
    ap: &ApEntryV1,
    ch: u8,
    ssid_watch: &SsidWatch,
    ssid_watch_cooldown: &mut HashMap<[u8; 6], u64>,
    ssid_watch_pending: &mut Vec<SsidWatchAlertV1>,
    sniffer_id: u32,
    sniffer_boot_id: u32,
) {
    if ssid_watch.is_empty() {
        return;
    }
    let ssid = ap_ssid_for_filter(ap);
    if ssid.is_empty() || !ssid_watch.matches(&ssid) {
        return;
    }
    let now_ms = wardriver_stats::now_ms();
    if let Some(&last) = ssid_watch_cooldown.get(&ap.bssid) {
        if now_ms.saturating_sub(last) < SSID_WATCH_COOLDOWN_MS {
            return;
        }
    }
    ssid_watch_cooldown.insert(ap.bssid, now_ms);
    ssid_watch_pending.push(SsidWatchAlertV1 {
        sniffer_id,
        sniffer_boot_id,
        channel: ch,
        rssi: ap.rssi,
        bssid: ap.bssid,
        ssid,
    });
}

/// Per-dwell promisc capture context: the per-dwell state plus the session-level state that
/// [`ingest_promisc_record`] reads or mutates. Built by [`PromiscCapture::capture_channel`]
/// for one dwell; the function body is unchanged except that parameters now arrive as struct
/// fields (same values, same order, same mutability).
struct CaptureSession<'a> {
    best: &'a mut HashMap<[u8; 6], ApEntryV1>,
    extras: &'a mut HashMap<[u8; 6], ScanEntryExtras>,
    bursts: &'a mut HashMap<[u8; 6], ProbeReqBurstAccum>,
    detection_bursts: &'a mut HashMap<[u8; 6], ProbeReqBurstAccum>,
    scan_only: bool,
    rec: PromiscRecord,
    expected_ch: u8,
    bssid_denylist: &'a HashSet<[u8; 6]>,
    ssid_denylist: &'a HashSet<String>,
    ssid_watch: &'a SsidWatch,
    start_ms32: u32,
    start_instant: Instant,
    camera: &'a mut LprCameraWifiDetector,
    remote_id: &'a mut RemoteIdDetector,
    sniffer_id: u32,
    detection_disable_wifi_mask: u32,
    remote_id_disable_wifi_mask: u32,
    wifi_needs_ie_sig: bool,
    wifi_needs_wildcard_probe: bool,
    wifi_needs_ms_oui: bool,
    alert_seq: u32,
    sniffer_boot_id: u32,
    alert_pending: &'a mut Vec<DetectionAlertV1>,
    remote_id_pending: &'a mut Vec<RemoteIdAlertV1>,
    alerted_probe_srcs: &'a mut HashSet<[u8; 6]>,
    ssid_watch_pending: &'a mut Vec<SsidWatchAlertV1>,
    ssid_watch_cooldown: &'a mut HashMap<[u8; 6], u64>,
    probe_records: &'a mut Vec<ProbeReqWireEntry>,
    probe_log: &'a mut HashMap<([u8; 6], [u8; 32], u8), ProbeLogEntry>,
}

fn ingest_promisc_record(ctx: &mut CaptureSession<'_>) {
    if ctx.rec.pkt_type != sys::wifi_promiscuous_pkt_type_t_WIFI_PKT_MGMT {
        return;
    }
    let pl = &ctx.rec.data[..ctx.rec.len as usize];
    let ch = if ctx.expected_ch != 0 {
        ctx.expected_ch
    } else {
        ctx.rec.channel
    };
    let t_ms32 = ctx.start_ms32.wrapping_add(
        ctx.start_instant
            .elapsed()
            .as_millis()
            .min(u128::from(u32::MAX)) as u32,
    );

    if is_remote_id_wifi_channel(ctx.expected_ch) && pl.len() >= 16 {
        let mut src = [0u8; 6];
        src.copy_from_slice(&pl[10..16]);
        let now_ms = wardriver_stats::now_ms();
        if let Some(alert) = ctx.remote_id.ingest_wifi(
            ctx.sniffer_id,
            now_ms,
            pl,
            src,
            ctx.rec.rssi,
            ch,
            ctx.remote_id_disable_wifi_mask,
        ) {
            let scan_end_ms = now_ms as u32;
            let scan_end_unix_ms = promisc_anchor_unix_ms();
            ctx.remote_id_pending.push(remote_id_alert_to_v1(
                &alert,
                ctx.alert_seq,
                ctx.sniffer_boot_id,
                scan_end_ms,
                scan_end_unix_ms,
            ));
        }
    }

    // Beacon / probe-response AP harvest (skipped in scan_only). SSID watch runs on beacons in both modes.
    if let Some(mut ap) = try_ap_from_mgmt_mpdu(pl, ctx.rec.rssi, ctx.rec.channel) {
        if ctx.expected_ch != 0 {
            ap.channel = ctx.expected_ch;
        }
        try_queue_ssid_watch_beacon(
            &ap,
            ch,
            ctx.ssid_watch,
            ctx.ssid_watch_cooldown,
            ctx.ssid_watch_pending,
            ctx.sniffer_id,
            ctx.sniffer_boot_id,
        );
        try_queue_ms_oui_alert(
            ctx.camera,
            ap.bssid,
            ch,
            ap.rssi,
            ctx.sniffer_id,
            ctx.detection_disable_wifi_mask,
            ctx.wifi_needs_ms_oui,
            false, // AP BSSID: Axon only; i-PRO is probe-request only
            ctx.alert_seq,
            ctx.sniffer_boot_id,
            ctx.alert_pending,
            ctx.alerted_probe_srcs,
        );
        if ctx.scan_only {
            return;
        }
        if ctx.bssid_denylist.contains(&ap.bssid) {
            return;
        }
        if ctx.ssid_denylist.contains(&ap_ssid_for_filter(&ap)) {
            return;
        }
        ap.obs_mono_ms = t_ms32;
        let bssid = ap.bssid;
        let new_ex = ScanEntryExtras {
            rcoi_ie_payload: roaming_consortium_ie_payload_from_mgmt_mpdu(pl).unwrap_or_default(),
            ble_cod: None,
        };
        match ctx.best.entry(bssid) {
            std::collections::hash_map::Entry::Occupied(mut occ) => {
                if ap.rssi > occ.get().rssi {
                    occ.insert(ap);
                    ctx.extras.insert(bssid, new_ex);
                }
            }
            std::collections::hash_map::Entry::Vacant(vac) => {
                vac.insert(ap);
                ctx.extras.insert(bssid, new_ex);
            }
        }
        return;
    }

    let Some(det) = try_probe_req_detail_from_mgmt_mpdu(pl, ctx.rec.rssi, ctx.rec.channel) else {
        return;
    };
    try_queue_ms_oui_alert(
        ctx.camera,
        det.src,
        ch,
        ctx.rec.rssi,
        ctx.sniffer_id,
        ctx.detection_disable_wifi_mask,
        ctx.wifi_needs_ms_oui,
        true, // probe-request src: Axon + i-PRO
        ctx.alert_seq,
        ctx.sniffer_boot_id,
        ctx.alert_pending,
        ctx.alerted_probe_srcs,
    );
    if det.is_wildcard_ssid {
        let burst_map = if ctx.scan_only {
            &mut *ctx.detection_bursts
        } else {
            &mut *ctx.bursts
        };
        let ie_sig_match = if !ctx.wifi_needs_ie_sig {
            false
        } else if burst_map
            .get(&det.src)
            .is_some_and(|a| a.primary_sig_matched)
        {
            true
        } else {
            wardriver_core::lpr_camera_probe_ie::probe_body_matches_lpr_camera_primary(&pl[24..])
        };
        let acc = burst_map
            .entry(det.src)
            .and_modify(|b| b.add(t_ms32, ctx.rec.rssi, ie_sig_match))
            .or_insert_with(|| ProbeReqBurstAccum::new(t_ms32, ch, ctx.rec.rssi, ie_sig_match));
        if ctx.wifi_needs_wildcard_probe {
            let sample = acc.to_sample(det.src);
            let now_ms = wardriver_stats::now_ms();
            let outcome = ctx.camera.eval_burst(
                ctx.sniffer_id,
                now_ms,
                &sample,
                ctx.detection_disable_wifi_mask,
            );
            if outcome.any() {
                ctx.alerted_probe_srcs.insert(det.src);
                if let Some(a) = pick_wifi_alert(&outcome) {
                    let scan_end_ms = now_ms as u32;
                    let scan_end_unix_ms = promisc_anchor_unix_ms();
                    ctx.alert_pending.push(wifi_alert_to_detection_alert_v1(
                        &a,
                        ctx.alert_seq,
                        ctx.sniffer_boot_id,
                        scan_end_ms,
                        scan_end_unix_ms,
                        acc.first_seen_ms32,
                        acc.last_seen_ms32,
                        acc.primary_sig_matched,
                    ));
                }
            }
        }
        return;
    }

    // Non-wildcard probe: capture SSID for hidden network discovery
    {
        let key = (det.src, det.ssid, det.ssid_len);
        let now_ms = wardriver_stats::now_ms() as u64;

        let should_log = match ctx.probe_log.get(&key) {
            None => true,                                                       // first sighting
            Some(e) if e.log_count >= PROBE_LOG_MAX_PER_ENTRY => false,         // row cap
            Some(e) if now_ms.saturating_sub(e.last_seen_ms) < 10_000 => false, // 10s cooldown
            Some(_) => true,
        };

        if should_log {
            let entry = ProbeReqWireEntry {
                src: det.src,
                ssid_len: det.ssid_len,
                ssid: det.ssid,
                channel: ch,
                rssi: ctx.rec.rssi,
            };
            ctx.probe_records.push(entry);

            ctx.probe_log
                .entry(key)
                .and_modify(|e| {
                    e.log_count = e.log_count.saturating_add(1);
                    e.last_seen_ms = now_ms;
                })
                .or_insert(ProbeLogEntry {
                    last_seen_ms: now_ms,
                    log_count: 1,
                });

            // Enforce dedup cap
            if ctx.probe_log.len() > PROBE_DEDUP_MAX_ENTRIES {
                if let Some(oldest_key) = ctx
                    .probe_log
                    .iter()
                    .min_by_key(|(_, e)| e.last_seen_ms)
                    .map(|(k, _)| *k)
                {
                    ctx.probe_log.remove(&oldest_key);
                }
            }
        }
    }
}

/// Promiscuous capture session that keeps promisc enabled across multiple channel dwells.
///
/// This reduces per-hop overhead vs toggling promisc for every channel.
pub struct PromiscCapture<'a> {
    rx: &'a Receiver<PromiscRecord>,
    camera: LprCameraWifiDetector,
    remote_id: RemoteIdDetector,
    sniffer_id: u32,
    detection_disable_wifi_mask: u32,
    remote_id_disable_wifi_mask: u32,
    wifi_needs_ie_sig: bool,
    wifi_needs_wildcard_probe: bool,
    wifi_needs_ms_oui: bool,
    alert_seq: u32,
    sniffer_boot_id: u32,
    scan_only: bool,
    alert_pending: Vec<DetectionAlertV1>,
    remote_id_pending: Vec<RemoteIdAlertV1>,
    ssid_watch_pending: Vec<SsidWatchAlertV1>,
    ssid_watch_cooldown: HashMap<[u8; 6], u64>,
    /// Source MACs that raised a detection alert during this capture session (merged into per-boot set by caller).
    session_alerted_srcs: HashSet<[u8; 6]>,
    /// Collected non-wildcard probe observations (drained after WiFi sweep, before BLE).
    probe_records: Vec<ProbeReqWireEntry>,
    /// Time-based dedup: (src MAC, ssid_bytes, ssid_len) → entry tracking last_seen and count.
    probe_log: HashMap<([u8; 6], [u8; 32], u8), ProbeLogEntry>,
}

impl<'a> PromiscCapture<'a> {
    pub fn begin(
        wifi: &mut EspWifi<'static>,
        rx: &'a Receiver<PromiscRecord>,
        sniffer_id: u32,
        detection_disable_wifi_mask: u32,
        remote_id_disable_wifi_mask: u32,
        alert_seq: u32,
        sniffer_boot_id: u32,
        scan_only: bool,
    ) -> Result<Self> {
        drain_queue(rx);
        set_mgmt_only_filters()?;
        wifi.driver_mut()
            .set_promiscuous(true)
            .map_err(|e| anyhow!("set_promiscuous(true): {e:?}"))?;
        Ok(Self {
            rx,
            camera: LprCameraWifiDetector::new(),
            remote_id: RemoteIdDetector::new(),
            sniffer_id,
            detection_disable_wifi_mask,
            remote_id_disable_wifi_mask,
            wifi_needs_ie_sig: wardriver_core::detection::wifi_mask_needs_ie_sig(
                detection_disable_wifi_mask,
            ),
            wifi_needs_wildcard_probe: wardriver_core::detection::wifi_mask_needs_wildcard_probe(
                detection_disable_wifi_mask,
            ),
            wifi_needs_ms_oui: wardriver_core::detection::wifi_mask_needs_ms_oui(
                detection_disable_wifi_mask,
            ),
            alert_seq,
            sniffer_boot_id,
            scan_only,
            alert_pending: Vec::new(),
            remote_id_pending: Vec::new(),
            ssid_watch_pending: Vec::new(),
            ssid_watch_cooldown: HashMap::new(),
            session_alerted_srcs: HashSet::new(),
            probe_records: Vec::new(),
            probe_log: HashMap::new(),
        })
    }

    /// Merge session alert MACs into the sniffer's per-boot follow-up allowlist.
    pub fn extend_alerted_probe_srcs(&self, into: &mut HashSet<[u8; 6]>) {
        into.extend(self.session_alerted_srcs.iter().copied());
    }

    /// Take pending immediate detection alerts from this capture session (clears the queue).
    pub fn take_alerts(&mut self) -> Vec<DetectionAlertV1> {
        std::mem::take(&mut self.alert_pending)
    }

    /// Take pending Remote ID alerts from this capture session (clears the queue).
    pub fn take_remote_id_alerts(&mut self) -> Vec<RemoteIdAlertV1> {
        std::mem::take(&mut self.remote_id_pending)
    }

    /// Take pending SSID watch alerts (clears the queue; does not abort the sweep).
    pub fn take_ssid_watch_alerts(&mut self) -> Vec<SsidWatchAlertV1> {
        std::mem::take(&mut self.ssid_watch_pending)
    }

    /// Drain collected probe request observation records (clears the internal buffer).
    /// Called after WiFi sweep completes, before BLE scan phase.
    pub fn take_probe_records(&mut self) -> Vec<ProbeReqWireEntry> {
        std::mem::take(&mut self.probe_records)
    }

    /// Passive capture on `expected_ch` for `dwell_ms`, returning unique AP entries (best RSSI wins).
    ///
    /// **Dwell time semantics:** This function applies the requested `dwell_ms` as **on-air listen
    /// time per channel**. The dwell timer starts immediately after the caller's `set_channel()`
    /// returns; it does not include the driver's channel-switch latency, which occurs before this
    /// timer begins. Total sweep duration across multiple channels follows the formula:
    /// `total_sweep = n_channels × dwell_ms + hop_overhead`, where `hop_overhead` accounts for
    /// WiFi driver switch time between channels.
    ///
    /// Assumes promisc is already enabled via [`PromiscCapture::begin`]. Returns AP beacon/probe
    /// response entries collected during the dwell window on `expected_ch`.
    pub fn capture_channel(
        &mut self,
        expected_ch: u8,
        dwell_ms: u16,
        bssid_denylist: &HashSet<[u8; 6]>,
        ssid_denylist: &HashSet<String>,
        ssid_watch: &SsidWatch,
    ) -> (
        Vec<ApEntryV1>,
        Vec<ScanEntryExtras>,
        Vec<ProbeReqBurstLocal>,
    ) {
        if self.scan_only {
            let mut detection_bursts: HashMap<[u8; 6], ProbeReqBurstAccum> = HashMap::new();
            let mut best: HashMap<[u8; 6], ApEntryV1> = HashMap::new();
            let mut extras: HashMap<[u8; 6], ScanEntryExtras> = HashMap::new();
            let mut bursts: HashMap<[u8; 6], ProbeReqBurstAccum> = HashMap::new();

            let start = Instant::now();
            let start_ms32 = wardriver_stats::now_ms() as u32;
            let deadline = start + Duration::from_millis(dwell_ms as u64);
            let detection_break = Cell::new(false);
            promisc_dwell_loop(
                self.rx,
                deadline,
                || detection_break.get(),
                |rec| {
                    wardriver_stats::queue_inflight_dec(
                        &wardriver_stats::PROMISC_RX_QUEUE_INFLIGHT,
                    );
                    let n_before = self.alert_pending.len();
                    let rid_before = self.remote_id_pending.len();
                    ingest_promisc_record(&mut CaptureSession {
                        best: &mut best,
                        extras: &mut extras,
                        bursts: &mut bursts,
                        detection_bursts: &mut detection_bursts,
                        scan_only: true,
                        rec,
                        expected_ch,
                        bssid_denylist,
                        ssid_denylist,
                        ssid_watch,
                        start_ms32,
                        start_instant: start,
                        camera: &mut self.camera,
                        remote_id: &mut self.remote_id,
                        sniffer_id: self.sniffer_id,
                        detection_disable_wifi_mask: self.detection_disable_wifi_mask,
                        remote_id_disable_wifi_mask: self.remote_id_disable_wifi_mask,
                        wifi_needs_ie_sig: self.wifi_needs_ie_sig,
                        wifi_needs_wildcard_probe: self.wifi_needs_wildcard_probe,
                        wifi_needs_ms_oui: self.wifi_needs_ms_oui,
                        alert_seq: self.alert_seq,
                        sniffer_boot_id: self.sniffer_boot_id,
                        alert_pending: &mut self.alert_pending,
                        remote_id_pending: &mut self.remote_id_pending,
                        alerted_probe_srcs: &mut self.session_alerted_srcs,
                        ssid_watch_pending: &mut self.ssid_watch_pending,
                        ssid_watch_cooldown: &mut self.ssid_watch_cooldown,
                        probe_records: &mut self.probe_records,
                        probe_log: &mut self.probe_log,
                    });
                    if self.alert_pending.len() > n_before
                        || self.remote_id_pending.len() > rid_before
                    {
                        detection_break.set(true);
                    }
                },
            );
            return (Vec::new(), Vec::new(), Vec::new());
        }

        let mut best: HashMap<[u8; 6], ApEntryV1> = HashMap::new();
        let mut extras: HashMap<[u8; 6], ScanEntryExtras> = HashMap::new();
        let mut bursts: HashMap<[u8; 6], ProbeReqBurstAccum> = HashMap::new();
        let mut detection_bursts: HashMap<[u8; 6], ProbeReqBurstAccum> = HashMap::new();

        let start = Instant::now();
        let start_ms32 = wardriver_stats::now_ms() as u32;
        let deadline = start + Duration::from_millis(dwell_ms as u64);
        let detection_break = Cell::new(false);
        promisc_dwell_loop(
            self.rx,
            deadline,
            || detection_break.get(),
            |rec| {
                wardriver_stats::queue_inflight_dec(&wardriver_stats::PROMISC_RX_QUEUE_INFLIGHT);
                let n_before = self.alert_pending.len();
                let rid_before = self.remote_id_pending.len();
                ingest_promisc_record(&mut CaptureSession {
                    best: &mut best,
                    extras: &mut extras,
                    bursts: &mut bursts,
                    detection_bursts: &mut detection_bursts,
                    scan_only: false,
                    rec,
                    expected_ch,
                    bssid_denylist,
                    ssid_denylist,
                    ssid_watch,
                    start_ms32,
                    start_instant: start,
                    camera: &mut self.camera,
                    remote_id: &mut self.remote_id,
                    sniffer_id: self.sniffer_id,
                    detection_disable_wifi_mask: self.detection_disable_wifi_mask,
                    remote_id_disable_wifi_mask: self.remote_id_disable_wifi_mask,
                    wifi_needs_ie_sig: self.wifi_needs_ie_sig,
                    wifi_needs_wildcard_probe: self.wifi_needs_wildcard_probe,
                    wifi_needs_ms_oui: self.wifi_needs_ms_oui,
                    alert_seq: self.alert_seq,
                    sniffer_boot_id: self.sniffer_boot_id,
                    alert_pending: &mut self.alert_pending,
                    remote_id_pending: &mut self.remote_id_pending,
                    alerted_probe_srcs: &mut self.session_alerted_srcs,
                    ssid_watch_pending: &mut self.ssid_watch_pending,
                    ssid_watch_cooldown: &mut self.ssid_watch_cooldown,
                    probe_records: &mut self.probe_records,
                    probe_log: &mut self.probe_log,
                });
                if self.alert_pending.len() > n_before || self.remote_id_pending.len() > rid_before
                {
                    detection_break.set(true);
                }
            },
        );

        let bursts_out: Vec<ProbeReqBurstLocal> = bursts
            .into_iter()
            .map(|(src, acc)| acc.into_burst(src))
            .collect();
        let mut entries = Vec::with_capacity(best.len());
        let mut extra_vec = Vec::with_capacity(best.len());
        for (bssid, ap) in best {
            extra_vec.push(extras.remove(&bssid).unwrap_or_default());
            entries.push(ap);
        }
        (entries, extra_vec, bursts_out)
    }

    /// Passive dwell on `expected_ch` while promiscuous mode is already enabled; invokes `cb`
    /// only for **802.11 probe request** management MPDUs (full frame bytes from IDF).
    pub fn dwell_probe_requests(
        &self,
        expected_ch: u8,
        dwell_ms: u16,
        mut cb: impl FnMut(&[u8], i8, u8),
    ) {
        let start = Instant::now();
        let deadline = start + Duration::from_millis(dwell_ms as u64);
        promisc_dwell_loop(
            self.rx,
            deadline,
            || false,
            |rec| {
                wardriver_stats::queue_inflight_dec(&wardriver_stats::PROMISC_RX_QUEUE_INFLIGHT);
                if rec.pkt_type != sys::wifi_promiscuous_pkt_type_t_WIFI_PKT_MGMT {
                    return;
                }
                let pl = &rec.data[..rec.len as usize];
                if !wardriver_core::ieee80211_parse::is_mgmt_probe_request(pl) {
                    return;
                }
                let ch = if expected_ch != 0 {
                    expected_ch
                } else {
                    rec.channel
                };
                cb(pl, rec.rssi, ch);
            },
        );
    }

    pub fn end(self, wifi: &mut EspWifi<'static>) -> Result<()> {
        wifi.driver_mut()
            .set_promiscuous(false)
            .map_err(|e| anyhow!("set_promiscuous(false): {e:?}"))?;
        drain_queue(self.rx);
        Ok(())
    }
}
