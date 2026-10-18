//! Host-testable sniffer lifecycle decisions (boot tracking, hello gating).

use std::collections::HashMap;

/// Tracks per-sniffer boot ids; detects reboot and signals state drop.
#[derive(Debug, Default)]
pub struct BootTracker {
    last_boot_id: HashMap<u32, u32>,
}

impl BootTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert boot id; returns true when id changed (sniffer reboot).
    pub fn note_boot(&mut self, sniffer_id: u32, boot_id: u32) -> bool {
        match self.last_boot_id.insert(sniffer_id, boot_id) {
            Some(prev) if prev != boot_id => true,
            _ => false,
        }
    }

    #[must_use]
    pub fn boot_id(&self, sniffer_id: u32) -> Option<u32> {
        self.last_boot_id.get(&sniffer_id).copied()
    }

    /// True when an uplink frame's boot id mismatches the tracked id (stale visit).
    #[must_use]
    pub fn uplink_boot_mismatch(&self, sniffer_id: u32, sniffer_boot_id: u32) -> bool {
        self.last_boot_id
            .get(&sniffer_id)
            .is_some_and(|tracked| *tracked != sniffer_boot_id)
    }
}

/// Outcome of roster + adoption gating for a Hello.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelloGate {
    DropNotRostered,
    Accept { fleet_slot: u8, rebooted: bool },
}

/// Pure roster gate for Hello (MAC+ID invariant).
#[must_use]
pub fn hello_gate(
    roster_allows: bool,
    fleet_slot: Option<u8>,
    rebooted: bool,
) -> Option<HelloGate> {
    if !roster_allows {
        return Some(HelloGate::DropNotRostered);
    }
    fleet_slot.map(|fleet_slot| HelloGate::Accept {
        fleet_slot,
        rebooted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_tracker_detects_reboot() {
        let mut t = BootTracker::new();
        assert!(!t.note_boot(1, 0x10));
        assert!(!t.note_boot(1, 0x10));
        assert!(t.note_boot(1, 0x20));
    }

    #[test]
    fn hello_gate_requires_roster_and_slot() {
        assert_eq!(
            hello_gate(false, Some(2), false),
            Some(HelloGate::DropNotRostered)
        );
        assert_eq!(hello_gate(true, None, true), None);
        assert_eq!(
            hello_gate(true, Some(3), true),
            Some(HelloGate::Accept {
                fleet_slot: 3,
                rebooted: true
            })
        );
    }
}
