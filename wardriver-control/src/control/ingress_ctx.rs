//! Per-tick inputs and mutable state for ESP-NOW frame dispatch.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, RwLock};

use indexmap::IndexMap;

use crate::alert_feedback::AlertHub;
use crate::config;
use crate::csv;
use crate::detection_alert_log::DetectionAlertLog;
use crate::detection_ble_detect::LprCameraBleDetector;
use crate::gps;
use crate::remote_id_alert_log::RemoteIdAlertLog;
use crate::storage;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use wardriver_core::lpr_camera_wifi::CameraAlertRecord;
use wardriver_shared::memory_policy::RuntimeMemoryPolicy;

use super::espnow_rx::{self, DedupKey, Reasm, ReasmBle, ReasmKey, ReasmProbe};

pub(crate) struct ControlIngressCtx<'a> {
    pub now_ms: u64,
    pub detection_dis_w: u32,
    pub detection_dis_b: u32,
    pub remote_id_dis: u32,
    pub control_boot_id: u32,
    pub scanning_allowed: &'a AtomicBool,
    pub cmd_seq: &'a AtomicU32,
    pub sniffers: &'a Arc<Mutex<HashSet<[u8; 6]>>>,
    pub nvs: &'a EspNvsPartition<NvsDefault>,
    pub cfg: &'a config::Config,
    pub last_sniffer_boot_id: &'a mut HashMap<u32, u32>,
    pub dedup: &'a mut IndexMap<DedupKey, u64>,
    pub dedup_ble: &'a mut IndexMap<DedupKey, u64>,
    pub dedup_probe: &'a mut IndexMap<DedupKey, u64>,
    pub reasm: &'a mut HashMap<ReasmKey, Reasm>,
    pub reasm_ble: &'a mut HashMap<ReasmKey, ReasmBle>,
    pub reasm_probe: &'a mut HashMap<ReasmKey, ReasmProbe>,
    pub mem_policy: &'a RuntimeMemoryPolicy,
    pub own_management_ap_bssid: Option<[u8; 6]>,
    pub wigle_home: &'a Arc<Mutex<Option<[u8; 6]>>>,
    pub wigle_manual: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    pub wigle_ssids_manual: &'a Arc<RwLock<HashSet<String>>>,
    pub detection_ignore: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    pub gps_hist: &'a VecDeque<(i64, gps::Fix)>,
    pub gps_fix: &'a Arc<Mutex<Option<gps::Fix>>>,
    pub sd_io: &'a storage::SdIoHandle,
    pub mac_table: &'a mut csv::MacTable,
    pub lpr_camera_ble: &'a mut LprCameraBleDetector,
    pub detection_alert_log_wifi: &'a mut DetectionAlertLog,
    pub detection_alert_log_ble: &'a mut DetectionAlertLog,
    pub remote_id_alert_log: &'a mut RemoteIdAlertLog,
    pub alert_hub: &'a Arc<AlertHub>,
    pub detection_last_alert: &'a mut Option<CameraAlertRecord>,
}
