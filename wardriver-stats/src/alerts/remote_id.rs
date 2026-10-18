//! Remote ID alert observability (`/api/stats` keys).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// Immediate Remote ID alerts received on ESP-NOW (`FrameType::RemoteIdAlert`).
pub static REMOTE_ID_ALERTS_RX: AtomicU32 = AtomicU32::new(0);
/// PACKcsv rows appended for Remote ID detections.
pub static REMOTE_ID_ROWS_WRITTEN: AtomicU32 = AtomicU32::new(0);

pub static REMOTE_ID_DISABLE_MASK: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteIdLast {
    pub last_seen_ms: u64,
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    pub signal_kind: u8,
    pub detection_method: u8,
    pub drone_lat_e7: i32,
    pub drone_lon_e7: i32,
}

static REMOTE_ID_LAST: Mutex<RemoteIdLast> = Mutex::new(RemoteIdLast {
    last_seen_ms: 0,
    sniffer_id: 0,
    src: [0; 6],
    channel: 0,
    rssi: 0,
    signal_kind: 0,
    detection_method: 0,
    drone_lat_e7: 0,
    drone_lon_e7: 0,
});

#[must_use]
pub fn remote_id_disable_mask() -> u32 {
    REMOTE_ID_DISABLE_MASK.load(Ordering::Acquire)
}

pub fn set_remote_id_disable_mask(mask: u32) {
    REMOTE_ID_DISABLE_MASK.store(mask, Ordering::Release);
}

pub fn record_remote_id_alert(last: RemoteIdLast) {
    if let Ok(mut g) = REMOTE_ID_LAST.lock() {
        *g = last;
    }
}

pub fn snapshot_remote_id_last() -> RemoteIdLast {
    REMOTE_ID_LAST.lock().map(|g| *g).unwrap_or_default()
}

pub fn add_remote_id_rows_written(n: u32) {
    REMOTE_ID_ROWS_WRITTEN.fetch_add(n, Ordering::Relaxed);
}
