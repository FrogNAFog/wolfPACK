//! FIFO pending-join queue for wolfPACK peer pool admission (host-testable).

/// Head loses priority without a `PeerJoin` refresh within this window.
pub const QUEUE_LIVENESS_MS: u64 = 30_000;
/// Default `PeerJoinNack::retry_after_ms` hint from control.
pub const NACK_RETRY_AFTER_MS: u16 = 1_000;
/// Visit-in-progress LRU guard when no bulk frag follows ack.
pub const VISIT_ARM_TIMEOUT_MS: u64 = 5_000;

/// Must match [`wardriver_stats::fleet::FLEET_MAX_SNIFFERS`].
pub const QUEUE_MAX_ENTRIES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueEntry {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub last_join_ms: u64,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitReason {
    NotYourTurn = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmitDecision {
    /// May call `ensure_peer` and send `PeerJoinAck`.
    Admit,
    /// Broadcast `PeerJoinNack`; do not ack.
    Wait {
        reason: WaitReason,
        /// 1 = FIFO head; 2+ = depth behind head.
        position: u8,
    },
}

#[derive(Debug, Default)]
pub struct PeerJoinQueue {
    entries: Vec<QueueEntry>,
}

impl PeerJoinQueue {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Live entries with a fresh `last_join_ms` (for Ping `peer_join_waiters`).
    #[must_use]
    pub fn live_waiter_count(&self, now_ms: u64) -> u8 {
        self.live_entries(now_ms).len().min(255) as u8
    }

    /// Remove stale heads; returns count removed (for stats).
    pub fn prune_expired(&mut self, now_ms: u64) -> u32 {
        let before = self.entries.len();
        self.entries
            .retain(|e| now_ms.saturating_sub(e.last_join_ms) <= QUEUE_LIVENESS_MS);
        (before.saturating_sub(self.entries.len())) as u32
    }

    /// Evaluate FIFO admit gate; enqueue when not admitted.
    pub fn admit_or_enqueue(
        &mut self,
        sniffer_id: u32,
        mac: [u8; 6],
        now_ms: u64,
    ) -> AdmitDecision {
        let _ = self.prune_expired(now_ms);

        // Repeat `PeerJoin` from a sniffer already waiting: refresh liveness, re-check head.
        if let Some(idx) = self.find_index(sniffer_id, &mac) {
            self.entries[idx].last_join_ms = now_ms;
            let position = (idx + 1) as u8;
            if idx == 0 {
                return AdmitDecision::Admit;
            }
            return AdmitDecision::Wait {
                reason: WaitReason::NotYourTurn,
                position,
            };
        }

        // New joiner: admit when no live waiters ahead.
        if self.entries.is_empty() {
            return AdmitDecision::Admit;
        }

        self.enqueue(sniffer_id, mac, now_ms);
        let position = self
            .find_index(sniffer_id, &mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1);
        AdmitDecision::Wait {
            reason: WaitReason::NotYourTurn,
            position,
        }
    }

    /// After successful `ensure_peer` + ack: drop from queue if present.
    pub fn on_admitted(&mut self, sniffer_id: u32, mac: &[u8; 6]) {
        if let Some(idx) = self.find_index(sniffer_id, mac) {
            self.entries.remove(idx);
        }
    }

    /// Drop all queue entries for a sniffer (stale liveness eviction hygiene).
    pub fn remove_sniffer(&mut self, sniffer_id: u32) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.sniffer_id != sniffer_id);
        before != self.entries.len()
    }

    /// Cap failure after admit gate passed: ensure waiter is tracked.
    pub fn enqueue_cap_full(&mut self, sniffer_id: u32, mac: [u8; 6], now_ms: u64) -> u8 {
        let _ = self.prune_expired(now_ms);
        if self.find_index(sniffer_id, &mac).is_some() {
            if let Some(idx) = self.find_index(sniffer_id, &mac) {
                self.entries[idx].last_join_ms = now_ms;
                return (idx + 1) as u8;
            }
        }
        self.enqueue(sniffer_id, mac, now_ms);
        self.find_index(sniffer_id, &mac)
            .map(|i| (i + 1) as u8)
            .unwrap_or(1)
    }

    fn live_entries(&self, now_ms: u64) -> Vec<&QueueEntry> {
        self.entries
            .iter()
            .filter(|e| now_ms.saturating_sub(e.last_join_ms) <= QUEUE_LIVENESS_MS)
            .collect()
    }

    fn find_index(&self, sniffer_id: u32, mac: &[u8; 6]) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.sniffer_id == sniffer_id && e.mac == *mac)
    }

    fn enqueue(&mut self, sniffer_id: u32, mac: [u8; 6], now_ms: u64) {
        if self.entries.len() >= QUEUE_MAX_ENTRIES {
            // Drop oldest tail when over cap (should be rare).
            self.entries.pop();
        }
        self.entries.push(QueueEntry {
            sniffer_id,
            mac,
            last_join_ms: now_ms,
        });
    }
}
