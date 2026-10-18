//! Control node: GPS + SD + WiGLE + ESP-NOW fleet hub.
//!
//! Split across submodules so ESP-NOW command fan-out and the main loop stay easier to navigate.

mod boot_run;
mod boot_setup;
mod channel_plan_cache;
mod control_workers;
mod csv_ingest;
mod csv_ingest_ble;
mod csv_ingest_detection_csv;
mod csv_ingest_gps;
mod csv_ingest_probe;
mod csv_ingest_wifi;
mod dedup_maintenance;
mod espnow_rx;
mod exclusion;
pub(crate) mod fleet_cmd;
pub(crate) mod fleet_fsm;
mod fleet_loop;
mod fleet_state;
mod ingest_shared;
mod ingress;
mod ingress_common;
mod ingress_ctx;
mod ingress_detection_alert;
mod ingress_detection_ble_alert;
mod ingress_detection_probe;
mod ingress_remote_id_alert;
mod ingress_sniffer;
mod ingress_ssid_watch;
mod main_loop;
mod main_loop_state;
mod nvs_defer_queue;
pub(crate) mod peer_pool;
mod run;
mod sd_bus_prep;
#[cfg(feature = "sd-pin-selftest")]
mod sd_pin_selftest;
pub(crate) mod time_sync_rtt;

/// SoftAP / ESP-NOW alignment channel (must match sniffer return channel).
pub(crate) const CONTROL_AP_CHANNEL: u8 = wardriver_proto::CONTROL_AP_CHANNEL;

pub use nvs_defer_queue::{NvsDeferOp, NvsDeferQueue, NvsDeferRequest, NVS_DEFER_WAIT_MS};
pub use run::run_control;
