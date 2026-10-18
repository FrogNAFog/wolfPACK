//! Shared firmware modules used by control and sniffer role binaries.

pub mod boot_button;
pub mod boot_diag;
pub mod cpu_load;
pub mod espnow;
pub mod fleet_mgmt;
pub mod memory_policy;
pub mod panic_boot;
pub mod pm_policy;
pub mod soc_temp;
pub mod spiram_heap;
pub mod spiram_queue;
pub mod spiram_thread;
pub mod task_wdt;
#[cfg(feature = "control")]
pub mod wifi_ap_tune;
pub mod wifi_regulatory;
pub mod wifi_tx_power;
