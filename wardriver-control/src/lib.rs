//! Control-node firmware: SoftAP, web dashboard, SD logging, GPS, and fleet coordination.

pub mod fleet_roster;

pub mod alert_feedback;
pub mod axon_detect;
#[cfg(feature = "board-xiao")]
pub mod buzzer;
pub mod channel_plan;
pub mod config;
pub mod control;
pub mod csv;
pub mod detection_alert_log;
pub mod detection_ble_detect;
mod detection_wifi_bridge;
pub mod display_ssd1306;
pub mod geo;
pub mod gps;
mod gps_recovery_log;
pub mod preferences;
pub mod provisioning;
pub mod recon;
pub mod remote_id_alert_log;
mod remote_id_bridge;
#[cfg(feature = "screen-st7920")]
pub mod screen_st7920;
pub mod sntp_sync;
pub mod storage;
pub mod time_sync;
pub mod uploader;
pub mod web;
pub mod wifi_ap_tune;
pub mod wifi_scan;

pub use control::run_control;

/// Control firmware entry (called from the root binary when `control` is enabled).
pub fn run() -> anyhow::Result<()> {
    gps_recovery_log::install();
    run_control()
}
