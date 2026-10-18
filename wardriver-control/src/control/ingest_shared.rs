//! Mutable scan-ingest state shared by the ESP-NOW ingest worker thread.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use super::csv_ingest::DETECTION_ALERT_LOG_CAP;
use super::espnow_rx::{DedupKey, Reasm, ReasmBle, ReasmKey, ReasmProbe};
use crate::csv;
use crate::detection_alert_log::DetectionAlertLog;
use crate::detection_ble_detect::LprCameraBleDetector;
use crate::remote_id_alert_log::RemoteIdAlertLog;
use wardriver_core::lpr_camera_wifi::CameraAlertRecord;
use wardriver_shared::memory_policy::RuntimeMemoryPolicy;
use wardriver_stats as stats;

pub(crate) struct IngestSharedState {
    pub lpr_camera_ble: LprCameraBleDetector,
    pub detection_last_alert: Option<CameraAlertRecord>,
    pub dedup: IndexMap<DedupKey, u64>,
    pub dedup_ble: IndexMap<DedupKey, u64>,
    pub dedup_probe: IndexMap<DedupKey, u64>,
    pub reasm: HashMap<ReasmKey, Reasm>,
    pub reasm_ble: HashMap<ReasmKey, ReasmBle>,
    pub reasm_probe: HashMap<ReasmKey, ReasmProbe>,
    pub mac_table: csv::MacTable,
    pub detection_alert_log_wifi: DetectionAlertLog,
    pub detection_alert_log_ble: DetectionAlertLog,
    pub remote_id_alert_log: RemoteIdAlertLog,
    pub last_sniffer_boot_id: HashMap<u32, u32>,
}

/// Publish ingest map sizes to dashboard atomics (called on `espnow-ingest` while holding the mutex).
pub(crate) fn publish_ingest_gauges(shared: &IngestSharedState) {
    stats::DEDUP_WIFI_SIZE.store(shared.dedup.len() as u32, Ordering::Relaxed);
    stats::DEDUP_BLE_SIZE.store(shared.dedup_ble.len() as u32, Ordering::Relaxed);
    stats::DEDUP_PROBE_SIZE.store(shared.dedup_probe.len() as u32, Ordering::Relaxed);
    stats::REASM_WIFI_SIZE.store(shared.reasm.len() as u32, Ordering::Relaxed);
    stats::REASM_BLE_SIZE.store(shared.reasm_ble.len() as u32, Ordering::Relaxed);
    stats::REASM_PROBE_SIZE.store(shared.reasm_probe.len() as u32, Ordering::Relaxed);
    stats::MAC_TABLE_ENTRIES.store(shared.mac_table.len() as u32, Ordering::Relaxed);
    stats::MAC_TABLE_GEO_ACTIVE.store(
        shared.mac_table.geo_active_count() as u32,
        Ordering::Relaxed,
    );
    stats::DETECTION_ALERTED_MACS_TRACKED.store(
        (shared.detection_alert_log_wifi.len() + shared.detection_alert_log_ble.len()) as u32,
        Ordering::Relaxed,
    );
}

impl IngestSharedState {
    pub(crate) fn new(mem_policy: &RuntimeMemoryPolicy) -> Self {
        Self {
            lpr_camera_ble: LprCameraBleDetector::new(),
            detection_last_alert: None,
            dedup: IndexMap::new(),
            dedup_ble: IndexMap::new(),
            dedup_probe: IndexMap::new(),
            reasm: HashMap::new(),
            reasm_ble: HashMap::new(),
            reasm_probe: HashMap::new(),
            mac_table: csv::MacTable::new(
                mem_policy.mac_table_max_entries,
                mem_policy.mac_table_geo_slots_max,
            ),
            detection_alert_log_wifi: DetectionAlertLog::new(DETECTION_ALERT_LOG_CAP),
            detection_alert_log_ble: DetectionAlertLog::new(DETECTION_ALERT_LOG_CAP),
            remote_id_alert_log: RemoteIdAlertLog::new(DETECTION_ALERT_LOG_CAP),
            last_sniffer_boot_id: HashMap::new(),
        }
    }
}
