//! Fleet size limits and SoftAP subnet shared by dashboard, ESP-NOW Kconfig, and
//! management SoftAP STA budget.

/// Control management SoftAP network base (192.168.71.0/24).
pub const CONTROL_SOFTAP_SUBNET: [u8; 4] = [192, 168, 71, 0];
/// Control SoftAP gateway (control node is always .1 on the fleet subnet).
pub const CONTROL_SOFTAP_GATEWAY: [u8; 4] = [192, 168, 71, 1];
pub const CONTROL_SOFTAP_NETMASK: [u8; 4] = [255, 255, 255, 0];

/// Management SoftAP slots for transient operator clients (dashboard phone/laptop).
/// Sniffers use ESP-NOW only; they do not associate to the SoftAP.
pub const SOFTAP_MAX_CONNECTIONS: u16 = 1;

/// Max adopted sniffers on the dashboard / NVS roster (single source: `wardriver-core`).
pub use wardriver_core::fleet_roster::FLEET_MAX_SNIFFERS;

/// Concurrent encrypted ESP-NOW peers (IDF `CONFIG_ESP_WIFI_ESPNOW_MAX_ENCRYPT_NUM`).
/// wolfPACK time-multiplexes visits; roster may exceed this cap.
pub const ESPNOW_MAX_ENCRYPT_PEERS: usize = 16;
