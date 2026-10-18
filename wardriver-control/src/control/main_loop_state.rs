//! Mutable control-loop state and per-tick dependency handles.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};

use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

use super::fleet_fsm::FleetOrchestrator;
use super::fleet_loop::FleetGateState;
use super::ingest_shared::IngestSharedState;
use super::nvs_defer_queue::NvsDeferRequest;

use crate::config;
use crate::gps;
use crate::storage;
use crate::web::snapshots::WebSnapshotState;
use wardriver_shared::memory_policy;

/// GPS soft-recovery throttle window (consumed by [`super::main_loop::GpsRecoveryPolicy`]).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct GpsRecoveryWindow {
    /// Soft-recovery attempts made in the current window.
    pub in_window: u32,
    /// Monotonic-ms timestamp the current window started (`0` = none started yet).
    pub window_start_ms: u64,
}

pub(crate) struct ControlLoopState {
    pub ingest: Arc<Mutex<IngestSharedState>>,
    /// Rolling GPS fixes for CSV row matching (main loop writes, worker threads read).
    pub gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    pub fleet_gate: FleetGateState,
    pub fleet: FleetOrchestrator,
    pub last_diag_log_ms: u64,
    pub last_rebalance_ms: u64,
    pub last_plan_revision: u32,
    pub last_active_sig: u64,
    pub last_gps_recovery_check_ms: u64,
    pub gps_recovery: GpsRecoveryWindow,
    pub last_periodic_ping_ms: u64,
    /// Throttle optional main-loop heartbeat diag lines (~5s).
    pub last_loop_heartbeat_ms: u64,
    pub channel_plan_sig: Vec<u32>,
    pub active_wifi_buf: Vec<(u32, [u8; 6])>,
    /// Last BOOT-hold countdown second shown on OLED (`board-xiao`).
    pub boot_unbind_last_oled_sec: Option<u64>,
}

impl ControlLoopState {
    pub(crate) fn new(
        gps_has_fix: bool,
        mem_policy: &memory_policy::RuntimeMemoryPolicy,
        gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    ) -> Self {
        Self {
            ingest: Arc::new(Mutex::new(IngestSharedState::new(mem_policy))),
            gps_hist,
            last_diag_log_ms: 0,
            last_rebalance_ms: 0,
            last_plan_revision: u32::MAX,
            last_active_sig: 0,
            fleet_gate: FleetGateState {
                gps_fix_ever_ok: gps_has_fix,
                ..FleetGateState::default()
            },
            fleet: FleetOrchestrator::default(),
            last_gps_recovery_check_ms: 0,
            gps_recovery: GpsRecoveryWindow::default(),
            last_periodic_ping_ms: 0,
            last_loop_heartbeat_ms: 0,
            channel_plan_sig: Vec::new(),
            active_wifi_buf: Vec::new(),
            boot_unbind_last_oled_sec: None,
        }
    }
}

/// Per-tick dependency handles, grouped so each main-loop sub-tick signature declares only the
/// slice of hardware / shared state it actually touches.
pub(crate) struct ControlLoopDeps<'a> {
    pub fleet: FleetDeps,
    pub exclusion: ExclusionDeps,
    pub gps: GpsDeps,
    pub core: CoreDeps<'a>,
}

/// Sniffer-fleet broadcast / command plumbing (prelude, core gates, schedule, tail, fleet FSM).
pub(crate) struct FleetDeps {
    pub sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    pub cmd_seq: Arc<AtomicU32>,
    pub control_boot_id: u32,
    pub own_management_ap_bssid: Option<[u8; 6]>,
    pub peer_if: u32,
    pub scanning_allowed: Arc<AtomicBool>,
}

/// Exclusion / denylist / watchlist sources and their dirty flags (prelude + core gates + snapshots).
pub(crate) struct ExclusionDeps {
    pub wigle_home: Arc<Mutex<Option<[u8; 6]>>>,
    pub wigle_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub wigle_ssids_manual: Arc<RwLock<HashSet<String>>>,
    pub detection_ignore: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub ssid_watchlist: Arc<RwLock<HashSet<String>>>,
    pub denylist_dirty: Arc<AtomicBool>,
    pub watchlist_dirty: Arc<AtomicBool>,
    pub detection_masks_dirty: Arc<AtomicBool>,
    pub home_geo: Arc<Mutex<config::HomeGeoConfig>>,
}

/// GPS fix + supervisor (core gates, GPS recovery sub-tick, snapshots).
pub(crate) struct GpsDeps {
    pub gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    pub gps_supervisor: Arc<gps::GpsSupervisor>,
}

/// Core hardware / services shared across the tick (prelude, core, fleet FSM, tail).
pub(crate) struct CoreDeps<'a> {
    pub wifi: &'a mut BlockingWifi<EspWifi<'static>>,
    pub nvs: &'a EspNvsPartition<NvsDefault>,
    pub alert_hub: Arc<crate::alert_feedback::AlertHub>,
    pub sd_io: Arc<storage::SdIoHandle>,
    pub nvs_defer_rx: Receiver<NvsDeferRequest>,
    pub web_snapshots: Arc<RwLock<WebSnapshotState>>,
}
