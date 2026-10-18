//! WiFi LPR-camera / mobile-surveillance detection from probe bursts and MAC OUI matches.
//!
//! Sniffers call [`LprCameraWifiDetector::eval_burst`] per qualifying wildcard observation and
//! [`LprCameraWifiDetector::eval_mac`] for mobile-surveillance OUI hits: method **4** (Axon) on
//! AP BSSIDs and probe-request sources; method **5** (i-PRO) on probe-request sources only
//! (`allow_ipro`).
//! The control node consumes [`CameraAlertRecord`] via ESP-NOW [`DetectionAlertV1`](wardriver_proto::DetectionAlertV1).

use std::collections::HashMap;

use crate::detection::{is_wifi_method_disabled, DetectionSignalKind, DetectionWifiMethod};
use crate::lpr_camera_ble::{is_axon_oui, is_ipro_oui};

/// Per-method cooldown (`0` allows back-to-back; production uses 30 s).
pub const PER_SRC_COOLDOWN_MS: u64 = 30_000;

const SRC_IDLE_EVICT_MS: u64 = 60_000;

/// One dwell-local wildcard probe burst sample (sniffer → detector).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraProbeSample {
    pub src: [u8; 6],
    pub wildcard_count: u8,
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub primary_sig_matched: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct CameraAlertRecord {
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub wildcard_count: u16,
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub oui_matched: bool,
    pub signal_kind: DetectionSignalKind,
    pub wifi_method: DetectionWifiMethod,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraProbeIngestOutcome {
    pub method1: Option<CameraAlertRecord>,
    pub method2: Option<CameraAlertRecord>,
    pub method3: Option<CameraAlertRecord>,
}

impl CameraProbeIngestOutcome {
    pub fn iter(&self) -> impl Iterator<Item = CameraAlertRecord> + '_ {
        [self.method1, self.method2, self.method3]
            .into_iter()
            .flatten()
    }

    pub fn any(&self) -> bool {
        self.method1.is_some() || self.method2.is_some() || self.method3.is_some()
    }
}

pub const LPR_CAMERA_OUIS: &[[u8; 3]] = &[
    [0x70, 0xC9, 0x4E],
    [0x3C, 0x91, 0x80],
    [0xD8, 0xF3, 0xBC],
    [0x80, 0x30, 0x49],
    [0xB8, 0x35, 0x32],
    [0x14, 0x5A, 0xFC],
    [0x74, 0x4C, 0xA1],
    [0x08, 0x3A, 0x88],
    [0x9C, 0x2F, 0x9D],
    [0xC0, 0x35, 0x32],
    [0x94, 0x08, 0x53],
    [0xE4, 0xAA, 0xEA],
    [0xF4, 0x6A, 0xDD],
    [0xF8, 0xA2, 0xD6],
    [0x24, 0xB2, 0xB9],
    [0x00, 0xF4, 0x8D],
    [0xD0, 0x39, 0x57],
    [0xE8, 0xD0, 0xFC],
    [0xE0, 0x4F, 0x43],
    [0xB8, 0x1E, 0xA4],
    [0x70, 0x08, 0x94],
    [0x58, 0x8E, 0x81],
    [0xEC, 0x1B, 0xBD],
    [0x3C, 0x71, 0xBF],
    [0x58, 0x00, 0xE3],
    [0x90, 0x35, 0xEA],
    [0x5C, 0x93, 0xA2],
    [0x64, 0x6E, 0x69],
    [0x48, 0x27, 0xEA],
    [0xA4, 0xCF, 0x12],
    [0x82, 0x6B, 0xF2],
];

fn is_lpr_camera_oui(src: &[u8; 6]) -> bool {
    let p = [src[0], src[1], src[2]];
    LPR_CAMERA_OUIS.iter().any(|o| *o == p)
}

/// True when `mac` matches the infrastructure OUI allowlist (WiFi methods 1/2 + BLE method 34).
#[must_use]
pub fn mac_matches_lpr_camera_oui(mac: &[u8; 6]) -> bool {
    is_lpr_camera_oui(mac)
}

#[derive(Clone, Copy, Debug, Default)]
struct SrcState {
    last_seen_ms: u64,
    /// Per-method last fire times for Wi-Fi methods **1..=5** (index = method_id − 1).
    last_alert_ms: [u64; 5],
}

fn cooldown_allows(st: &SrcState, method: DetectionWifiMethod, now_ms: u64) -> bool {
    let idx = method.as_u8().saturating_sub(1) as usize;
    if idx >= st.last_alert_ms.len() {
        return true;
    }
    let last = st.last_alert_ms[idx];
    last == 0 || now_ms.saturating_sub(last) >= PER_SRC_COOLDOWN_MS
}

fn mark_fired(st: &mut SrcState, method: DetectionWifiMethod, now_ms: u64) {
    let idx = method.as_u8().saturating_sub(1) as usize;
    if idx < st.last_alert_ms.len() {
        st.last_alert_ms[idx] = now_ms;
    }
}

fn build_alert(
    sniffer_id: u32,
    sample: &CameraProbeSample,
    oui_matched: bool,
    wifi_method: DetectionWifiMethod,
) -> CameraAlertRecord {
    CameraAlertRecord {
        sniffer_id,
        src: sample.src,
        wildcard_count: sample.wildcard_count as u16,
        channel: sample.channel,
        rssi_min: sample.rssi_min,
        rssi_max: sample.rssi_max,
        rssi_avg_q8: sample.rssi_avg_q8,
        oui_matched,
        signal_kind: DetectionSignalKind::Wifi,
        wifi_method,
    }
}

/// Shared cooldown+fire core: gate on the method's enable mask and a per-method
/// `gate` (OUI / IE-signature / wildcard eligibility), then mark fired + build the alert.
fn try_fire(
    st: &mut SrcState,
    sniffer_id: u32,
    sample: &CameraProbeSample,
    now_ms: u64,
    disable_wifi_mask: u32,
    method: DetectionWifiMethod,
    gate: bool,
    oui_matched: bool,
) -> Option<CameraAlertRecord> {
    if is_wifi_method_disabled(disable_wifi_mask, method.as_u8()) || !gate {
        return None;
    }
    if !cooldown_allows(st, method, now_ms) {
        return None;
    }
    mark_fired(st, method, now_ms);
    Some(build_alert(sniffer_id, sample, oui_matched, method))
}

fn try_alert_for_method(
    st: &mut SrcState,
    sniffer_id: u32,
    sample: &CameraProbeSample,
    now_ms: u64,
    disable_wifi_mask: u32,
    method: DetectionWifiMethod,
    extra_ok: bool,
    oui_matched: bool,
) -> Option<CameraAlertRecord> {
    if sample.wildcard_count == 0 {
        return None;
    }
    try_fire(
        st,
        sniffer_id,
        sample,
        now_ms,
        disable_wifi_mask,
        method,
        extra_ok,
        oui_matched,
    )
}

fn try_ms_oui_alert(
    st: &mut SrcState,
    sniffer_id: u32,
    mac: [u8; 6],
    channel: u8,
    rssi: i8,
    now_ms: u64,
    disable_wifi_mask: u32,
    method: DetectionWifiMethod,
    oui_ok: bool,
) -> Option<CameraAlertRecord> {
    let sample = CameraProbeSample {
        src: mac,
        wildcard_count: 0,
        channel,
        rssi_min: rssi,
        rssi_max: rssi,
        rssi_avg_q8: (rssi as i16) << 8,
        primary_sig_matched: false,
    };
    try_fire(
        st,
        sniffer_id,
        &sample,
        now_ms,
        disable_wifi_mask,
        method,
        oui_ok,
        true,
    )
}

pub struct LprCameraWifiDetector {
    per_src: HashMap<[u8; 6], SrcState>,
}

impl Default for LprCameraWifiDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl LprCameraWifiDetector {
    pub fn new() -> Self {
        debug_assert!(!LPR_CAMERA_OUIS.is_empty());
        Self {
            per_src: HashMap::new(),
        }
    }

    pub fn prune(&mut self, now_ms: u64) {
        self.per_src
            .retain(|_, s| now_ms.saturating_sub(s.last_seen_ms) <= SRC_IDLE_EVICT_MS);
    }

    /// Evaluate methods 1–3 on a single sample (no rolling-window merge).
    pub fn eval_burst(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        sample: &CameraProbeSample,
        disable_wifi_mask: u32,
    ) -> CameraProbeIngestOutcome {
        let st = self.per_src.entry(sample.src).or_default();
        st.last_seen_ms = now_ms;

        if sample.wildcard_count == 0 {
            return CameraProbeIngestOutcome::default();
        }

        let oui_ok = is_lpr_camera_oui(&sample.src);
        let ie_sig_ok = sample.primary_sig_matched;

        let m1 = try_alert_for_method(
            st,
            sniffer_id,
            sample,
            now_ms,
            disable_wifi_mask,
            DetectionWifiMethod::WildcardProbesOui,
            oui_ok,
            oui_ok,
        );
        let m2 = try_alert_for_method(
            st,
            sniffer_id,
            sample,
            now_ms,
            disable_wifi_mask,
            DetectionWifiMethod::WildcardProbeIeSignatureOui,
            oui_ok && ie_sig_ok,
            oui_ok,
        );
        let m3 = try_alert_for_method(
            st,
            sniffer_id,
            sample,
            now_ms,
            disable_wifi_mask,
            DetectionWifiMethod::WildcardProbeIeSignatureAnyMac,
            ie_sig_ok,
            oui_ok,
        );

        CameraProbeIngestOutcome {
            method1: m1,
            method2: m2,
            method3: m3,
        }
    }

    /// Evaluate mobile-surveillance Wi-Fi OUI methods (**4** Axon, **5** i-PRO) on a MAC.
    ///
    /// Method **4** (Axon) applies to beacon / probe-response BSSIDs and probe-request sources.
    /// Method **5** (i-PRO) runs only when `allow_ipro` is true (probe-request sources).
    /// Does not require wildcard probes or IE signatures. Prefers Axon over i-PRO when both
    /// would match (OUIs are disjoint in practice).
    pub fn eval_mac(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        mac: [u8; 6],
        channel: u8,
        rssi: i8,
        disable_wifi_mask: u32,
        allow_ipro: bool,
    ) -> Option<CameraAlertRecord> {
        let st = self.per_src.entry(mac).or_default();
        st.last_seen_ms = now_ms;

        if let Some(a) = try_ms_oui_alert(
            st,
            sniffer_id,
            mac,
            channel,
            rssi,
            now_ms,
            disable_wifi_mask,
            DetectionWifiMethod::AxonOui,
            is_axon_oui(&mac),
        ) {
            return Some(a);
        }
        if !allow_ipro {
            return None;
        }
        try_ms_oui_alert(
            st,
            sniffer_id,
            mac,
            channel,
            rssi,
            now_ms,
            disable_wifi_mask,
            DetectionWifiMethod::IproOui,
            is_ipro_oui(&mac),
        )
    }
}
