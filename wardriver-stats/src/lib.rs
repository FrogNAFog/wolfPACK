//! Telemetry counters and stats aggregation for dashboard and diagnostics.
//!
//! Global runtime statistics (`counters`) plus alert-observability submodules
//! (`alerts/*`), the fleet size limits, and the tier-A `/api/stats` snapshot
//! types and builders.

mod alerts;
mod counters;
mod diagnostics;
pub mod fleet;
mod host_stub;
pub mod json_fixed;
mod snapshot;
#[cfg(feature = "control")]
pub mod stats_body;
mod tier_a_build;

pub use alerts::alert_kinds::*;
pub use alerts::detection::*;
pub use alerts::gps::*;
pub use alerts::remote_id::*;
pub use alerts::sniffer_slots::*;
pub use alerts::ssid_watch::*;
pub use alerts::upload_progress::*;
pub use alerts::*;
pub use counters::*;
pub use diagnostics::*;
pub use fleet::*;
pub use json_fixed::*;
pub use snapshot::*;
#[cfg(feature = "control")]
pub use stats_body::*;
