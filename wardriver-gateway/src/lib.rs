//! Minimal ESP32-C5 wolfPACK gateway: ESP-NOW coprocessor + PackLink SoftAP + PSRAM spool.

pub mod adoption_retry;
pub mod capture_job;
pub mod channel_plan_downlink;
pub mod espnow_bridge;
pub mod fleet_loop;
pub mod gateway_adoption;
pub mod gateway_ingress;
pub mod gateway_led;
pub mod gateway_softap;
pub mod ingest_state;
pub mod packlink_nvs_defer;
pub mod packlink_trust;
pub mod policy_downlink;
pub mod policy_store;
pub mod psram_spool;
pub mod run;
pub mod softap_transport;

pub use run::run_gateway;

/// Gateway firmware entry (root binary when `gateway` feature is enabled).
pub fn run() -> anyhow::Result<()> {
    run_gateway()
}
