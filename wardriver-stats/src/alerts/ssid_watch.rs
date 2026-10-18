//! SSID watch alert observability (`/api/stats` keys for dashboard banner).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// Beacon SSID watch alerts fired on control (no separate SD log).
pub static SSID_WATCH_ALERTS_FIRED: AtomicU32 = AtomicU32::new(0);

/// Last SSID watch hit for fleet dashboard banner.
#[derive(Clone, Debug, Default)]
pub struct SsidWatchLast {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub bssid: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    /// Matched SSID string (≤32 UTF-8 bytes on wire).
    pub ssid: String,
}

static SSID_WATCH_LAST: Mutex<SsidWatchLast> = Mutex::new(SsidWatchLast {
    last_seen_ms: 0,
    sniffer_id: 0,
    bssid: [0; 6],
    channel: 0,
    rssi: 0,
    ssid: String::new(),
});

pub fn record_ssid_watch_alert(last: SsidWatchLast) {
    if let Ok(mut g) = SSID_WATCH_LAST.lock() {
        *g = last;
    }
    SSID_WATCH_ALERTS_FIRED.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot_ssid_watch_last() -> SsidWatchLast {
    SSID_WATCH_LAST
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}
