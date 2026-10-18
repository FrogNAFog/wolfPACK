//! wolfPACK fleet realtime: peer pool, ESP-NOW downlink.
//!
//! Timing-critical wolfPACK paths shared by control node and BLE gateway coprocessor.

pub mod channel_plan_runtime;
pub mod fleet_commands;
pub mod fleet_ingress_gate;
pub mod fleet_tick;
pub mod frame_router;
pub mod rebalance_policy;
pub mod roster_gate;
pub mod sniffer_lifecycle;
pub mod welcome_bundle;

#[cfg(feature = "esp")]
pub mod gateway_downlink;
#[cfg(feature = "esp")]
pub mod ingest;
#[cfg(feature = "esp")]
pub mod peer_pool;
#[cfg(feature = "esp")]
pub mod sniffer_ingress;
#[cfg(feature = "esp")]
pub mod time_sync_rtt;
#[cfg(feature = "esp")]
pub mod welcome_downlink;

pub use fleet_ingress_gate::{bypass_roster_gate, ingress_roster_should_drop};
pub use fleet_tick::{periodic_tick_should_run, queue_hint_tick_should_run, FleetTickState};
pub use roster_gate::{roster_allows, roster_fleet_slot, PolicySyncV1, RosterEntryV1};

#[cfg(feature = "esp")]
pub use fleet_tick::{maybe_tick_queue_hint_ping, tick_fleet_ping, FleetTickDeps};

/// SoftAP / ESP-NOW alignment channel (must match sniffer return channel).
pub const CONTROL_AP_CHANNEL: u8 = wardriver_proto::CONTROL_AP_CHANNEL;

/// ESP-NOW peer interface for APSTA control/gateway (encrypted unicast on SoftAP path).
#[cfg(feature = "esp")]
pub use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;
