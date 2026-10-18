//! LPR-camera and mobile-surveillance alert observability (PACK-aligned).

use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

// -------- LPR-camera detection (probe-request behavior) --------
//
// Control node only produces these values (sniffer builds will keep defaults).

/// 1 when at least one WiFi method (1–3) or BLE method (32–35) is not fully masked off; else 0.
pub static DETECTION_ENABLED: AtomicU8 = AtomicU8::new(0);

/// 0=none, 1–2 reserved (legacy WiFi sniffer count reasons), 3=all methods masked off.
pub static DETECTION_DISABLED_REASON: AtomicU8 = AtomicU8::new(0);
pub const DETECTION_DISABLED_NONE: u8 = 0;
pub const DETECTION_DISABLED_ALL_METHODS_MASKED: u8 = 3;

pub static DETECTION_PROBE_FRAGS_RX: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_BURSTS_RX: AtomicU32 = AtomicU32::new(0);
/// Immediate LPR-camera WiFi alerts received on ESP-NOW (`FrameType::DetectionAlert`).
pub static DETECTION_ALERTS_RX: AtomicU32 = AtomicU32::new(0);
/// WiFi-side detection alerts (LPR-camera WiFi methods 1–3); each fire increments this (not follow-up rows).
pub static DETECTION_ALERTS_FIRED: AtomicU32 = AtomicU32::new(0);
/// Per-WiFi-method alert counters (one row per method per ingest call). Pack parity.
pub static DETECTION_WIFI_ALERTS_FIRED_METHOD1: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_WIFI_ALERTS_FIRED_METHOD2: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_WIFI_ALERTS_FIRED_METHOD3: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_WIFI_IE_SIG_MATCH_BUILTIN_TOTAL: AtomicU32 = AtomicU32::new(0);
/// BLE-side LPR-camera infrastructure alerts (methods 32–34).
pub static BLE_ALERTS_FIRED: AtomicU32 = AtomicU32::new(0);
/// Mobile surveillance BLE alerts (PACK Axon OUI, method **35**).
pub static MOBILE_SURVEILLANCE_ALERTS_FIRED: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_FOLLOWUP_ROWS_WRITTEN: AtomicU32 = AtomicU32::new(0);
pub static DETECTION_ALERTED_MACS_TRACKED: AtomicU32 = AtomicU32::new(0);

pub static DETECTION_DISABLE_WIFI_MASK: AtomicU32 =
    AtomicU32::new(wardriver_core::detection::DETECTION_WIFI_DISABLE_MASK_DEFAULT);
pub static DETECTION_DISABLE_BLE_MASK: AtomicU32 =
    AtomicU32::new(wardriver_core::detection::DETECTION_BLE_DISABLE_MASK_DEFAULT);

#[derive(Clone, Copy, Debug, Default)]
pub struct DetectionLast {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub wildcard_count: u16,
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub oui_matched: u8,
    pub signal_kind: u8,
    pub detection_method: u8,
    pub ble_company_id: u16,
}

static DETECTION_LAST: Mutex<DetectionLast> = Mutex::new(DetectionLast {
    last_seen_ms: 0,
    sniffer_id: 0,
    src: [0; 6],
    wildcard_count: 0,
    channel: 0,
    rssi_min: 0,
    rssi_max: 0,
    rssi_avg_q8: 0,
    oui_matched: 0,
    signal_kind: 0,
    detection_method: 0,
    ble_company_id: 0,
});

/// Last mobile-surveillance alert snapshot (`/api/stats` `mobile_surveillance_last_*` keys).
#[derive(Clone, Copy, Debug, Default)]
pub struct MobileSurveillanceLast {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub signal_kind: u8,
    pub detection_method: u8,
}

static MOBILE_SURVEILLANCE_LAST: Mutex<MobileSurveillanceLast> =
    Mutex::new(MobileSurveillanceLast {
        last_seen_ms: 0,
        sniffer_id: 0,
        src: [0; 6],
        signal_kind: 0,
        detection_method: 0,
    });

pub fn set_detection_enabled(enabled: bool, disabled_reason: u8) {
    DETECTION_ENABLED.store(if enabled { 1 } else { 0 }, Ordering::Release);
    DETECTION_DISABLED_REASON.store(disabled_reason, Ordering::Release);
}

#[must_use]
pub fn detection_disable_wifi_mask() -> u32 {
    DETECTION_DISABLE_WIFI_MASK.load(Ordering::Acquire)
}

#[must_use]
pub fn detection_disable_ble_mask() -> u32 {
    DETECTION_DISABLE_BLE_MASK.load(Ordering::Acquire)
}

pub fn set_detection_disable_masks(wifi: u32, ble: u32) {
    DETECTION_DISABLE_WIFI_MASK.store(wifi, Ordering::Release);
    DETECTION_DISABLE_BLE_MASK.store(ble, Ordering::Release);
}

pub fn record_detection_alert(last: DetectionLast) {
    if let Ok(mut g) = DETECTION_LAST.lock() {
        *g = last;
    }
    if last.signal_kind == 1 {
        BLE_ALERTS_FIRED.fetch_add(1, Ordering::Relaxed);
    } else {
        DETECTION_ALERTS_FIRED.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_detection_wifi_method_alert(method_id: u8) {
    match method_id {
        1 => {
            DETECTION_WIFI_ALERTS_FIRED_METHOD1.fetch_add(1, Ordering::Relaxed);
        }
        2 => {
            DETECTION_WIFI_ALERTS_FIRED_METHOD2.fetch_add(1, Ordering::Relaxed);
        }
        3 => {
            DETECTION_WIFI_ALERTS_FIRED_METHOD3.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

pub fn snapshot_detection_last() -> DetectionLast {
    DETECTION_LAST.lock().map(|g| *g).unwrap_or_default()
}

pub fn record_mobile_surveillance_alert(last: MobileSurveillanceLast) {
    if let Ok(mut g) = MOBILE_SURVEILLANCE_LAST.lock() {
        *g = last;
    }
    MOBILE_SURVEILLANCE_ALERTS_FIRED.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot_mobile_surveillance_last() -> MobileSurveillanceLast {
    MOBILE_SURVEILLANCE_LAST
        .lock()
        .map(|g| *g)
        .unwrap_or_default()
}
