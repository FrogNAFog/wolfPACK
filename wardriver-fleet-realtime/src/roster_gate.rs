//! Phone-driven roster gate (`PolicySyncV1` RAM policy).

pub use wardriver_packlink::roster_allows;
pub use wardriver_packlink::{PolicySyncV1, RosterEntryV1};

/// Stable dashboard slot for a rostered sniffer (1..=N).
#[must_use]
pub fn roster_fleet_slot(policy: &PolicySyncV1, sniffer_id: u32) -> Option<u8> {
    policy
        .roster
        .iter()
        .find(|e| e.sniffer_id == sniffer_id)
        .map(|e| e.fleet_slot)
}
