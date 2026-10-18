//! Sniffer firmware: WiFi promiscuous capture, BLE observation, and ESP-NOW reporting.

mod ble_sniffer;
#[cfg(feature = "ble")]
mod ble_stack;
mod remote_id_alert_wire;
mod sniffer;
mod sniffer_ble;
mod sniffer_config;
mod sniffer_dispatch;
mod sniffer_espnow;
mod sniffer_led;
mod sniffer_link;
mod sniffer_outbound_bulk;
mod sniffer_recon;
mod sniffer_sweep;
mod sniffer_sweep_finish;
mod sniffer_wifi;
mod wifi_promisc;

/// Sniffer firmware entry (called from the root binary when `sniffer` is enabled).
pub fn run() -> anyhow::Result<()> {
    sniffer::run_sniffer()
}
