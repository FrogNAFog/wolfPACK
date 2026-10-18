//! Gateway-only adoption retry state (host-testable; no ESP-NOW dependency).
//!
//! Tracks pending roster binds until a matching [`SnifferAck`] confirms the
//! follow-on Start/Stop `cmd_seq` from `ControlWelcome`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use wardriver_fleet_realtime::welcome_bundle::WelcomeAttemptResult;

/// Retry window for a single adopt attempt series.
pub const ADOPT_RETRY_DEADLINE_MS: u64 = 60_000;

/// Recent Hello within this window counts as "still hearing" the sniffer.
pub const ADOPT_HELLO_LIVE_MS: u64 = 10_000;

/// Cap parallel welcome sends per fleet tick to reduce encrypted peer churn.
pub const MAX_WELCOME_ATTEMPTS_PER_TICK: usize = 2;

/// Pause after `ReleaseBinding` before `ControlWelcome`.
pub const RELEASE_BEFORE_WELCOME_MS: u64 = 250;

const BACKOFF_MS: [u64; 5] = [250, 500, 1_000, 2_000, 5_000];

/// Gateway-reported adoption progress for PackLink `SnifferStatus` adopt tail.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdoptProgress {
    None = 0,
    WaitingPolicy = 1,
    RetryingWelcome = 2,
    Confirmed = 3,
    TimedOutHelloLive = 4,
    TimedOutNoHello = 5,
}

/// Gateway-reported adoption failure/diagnostic reason.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdoptReason {
    None = 0,
    SendFailed = 1,
    PossibleExistingBinding = 2,
    RosterRemoved = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingAdopt {
    sniffer_id: u32,
    mac: [u8; 6],
    fleet_slot: u8,
    attempt_count: u32,
    expected_ack_cmd_seq: Option<u32>,
    armed_at_ms: u64,
    next_attempt_ms: u64,
    last_hello_ms: u64,
    last_send_failed: bool,
    confirmed: bool,
    timed_out: bool,
    timed_out_hello_live: bool,
    release_before_welcome: bool,
    release_sent_ms: u64,
}

impl PendingAdopt {
    fn progress(&self, now_ms: u64) -> AdoptProgress {
        if self.confirmed {
            return AdoptProgress::Confirmed;
        }
        if self.timed_out {
            return if self.timed_out_hello_live {
                AdoptProgress::TimedOutHelloLive
            } else {
                AdoptProgress::TimedOutNoHello
            };
        }
        if self.expected_ack_cmd_seq.is_some() || self.attempt_count > 0 {
            return AdoptProgress::RetryingWelcome;
        }
        if self.last_hello_ms > 0 && now_ms.saturating_sub(self.last_hello_ms) < ADOPT_HELLO_LIVE_MS
        {
            return AdoptProgress::RetryingWelcome;
        }
        if now_ms.saturating_sub(self.armed_at_ms) < ADOPT_RETRY_DEADLINE_MS {
            return AdoptProgress::WaitingPolicy;
        }
        AdoptProgress::None
    }

    fn reason(&self) -> AdoptReason {
        if self.confirmed || !self.timed_out {
            if self.last_send_failed && !self.confirmed && !self.timed_out {
                return AdoptReason::SendFailed;
            }
            return AdoptReason::None;
        }
        if self.timed_out_hello_live {
            return AdoptReason::PossibleExistingBinding;
        }
        AdoptReason::None
    }
}

#[derive(Default)]
struct AdoptionStore {
    pending: HashMap<u32, PendingAdopt>,
}

static STORE: OnceLock<Mutex<AdoptionStore>> = OnceLock::new();

fn store() -> &'static Mutex<AdoptionStore> {
    STORE.get_or_init(|| Mutex::new(AdoptionStore::default()))
}

fn backoff_ms(attempt_count: u32) -> u64 {
    BACKOFF_MS
        .get(attempt_count as usize)
        .copied()
        .unwrap_or(5_000)
}

/// Arm or re-arm adoption for a roster entry (PolicySync apply / manual retry).
pub fn arm(sniffer_id: u32, mac: [u8; 6], fleet_slot: u8, now_ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(mut g) = store().lock() else {
        return;
    };
    match g.pending.get_mut(&sniffer_id) {
        Some(entry) if entry.confirmed => {
            if entry.mac != mac || entry.fleet_slot != fleet_slot {
                entry.confirmed = false;
                entry.mac = mac;
                entry.fleet_slot = fleet_slot;
                entry.expected_ack_cmd_seq = None;
                entry.attempt_count = 0;
                entry.last_send_failed = false;
                entry.timed_out = false;
                entry.timed_out_hello_live = false;
                entry.armed_at_ms = now_ms;
                entry.next_attempt_ms = now_ms;
                entry.release_before_welcome = true;
                entry.release_sent_ms = 0;
            }
        }
        Some(entry) => {
            let identity_changed = entry.mac != mac || entry.fleet_slot != fleet_slot;
            entry.mac = mac;
            entry.fleet_slot = fleet_slot;
            if entry.timed_out || identity_changed {
                entry.timed_out = false;
                entry.timed_out_hello_live = false;
                entry.last_send_failed = false;
                entry.armed_at_ms = now_ms;
                entry.next_attempt_ms = now_ms;
                entry.expected_ack_cmd_seq = None;
                entry.release_sent_ms = 0;
                // Release only on identity takeover; timeout re-arm is welcome-only.
                entry.release_before_welcome = identity_changed;
                if identity_changed {
                    entry.attempt_count = 0;
                }
            }
        }
        None => {
            g.pending.insert(
                sniffer_id,
                PendingAdopt {
                    sniffer_id,
                    mac,
                    fleet_slot,
                    attempt_count: 0,
                    expected_ack_cmd_seq: None,
                    armed_at_ms: now_ms,
                    next_attempt_ms: now_ms,
                    last_hello_ms: 0,
                    last_send_failed: false,
                    confirmed: false,
                    timed_out: false,
                    timed_out_hello_live: false,
                    release_before_welcome: true,
                    release_sent_ms: 0,
                },
            );
        }
    }
}

/// Cancel adoption retries for a removed roster entry.
pub fn cancel(sniffer_id: u32) {
    if sniffer_id == 0 {
        return;
    }
    if let Ok(mut g) = store().lock() {
        g.pending.remove(&sniffer_id);
    }
}

/// Rostered sniffer sent `SnifferHello`.
pub fn note_hello(sniffer_id: u32, now_ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(mut g) = store().lock() else {
        return;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return;
    };
    if entry.confirmed {
        return;
    }
    entry.last_hello_ms = now_ms;
    if entry.timed_out {
        entry.timed_out = false;
        entry.timed_out_hello_live = false;
        entry.armed_at_ms = now_ms;
        entry.next_attempt_ms = now_ms;
        // Welcome-only retry; do not ReleaseBinding on timeout reopen.
        entry.release_before_welcome = false;
        entry.release_sent_ms = 0;
    } else if now_ms >= entry.next_attempt_ms {
        entry.next_attempt_ms = now_ms;
    }
}

/// Bump retry scheduling so the next fleet tick can send welcome promptly.
pub fn request_welcome_now(sniffer_id: u32, now_ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(mut g) = store().lock() else {
        return;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return;
    };
    if entry.confirmed || entry.timed_out {
        return;
    }
    entry.next_attempt_ms = now_ms;
}

/// True when a `ReleaseBinding` unicast should precede the next welcome attempt.
#[must_use]
pub fn needs_release_send(sniffer_id: u32) -> bool {
    if sniffer_id == 0 {
        return false;
    }
    let Ok(g) = store().lock() else {
        return false;
    };
    g.pending
        .get(&sniffer_id)
        .is_some_and(|e| !e.confirmed && e.release_before_welcome && e.release_sent_ms == 0)
}

/// Record that `ReleaseBinding` was sent; welcome is deferred for [`RELEASE_BEFORE_WELCOME_MS`].
pub fn note_release_sent(sniffer_id: u32, now_ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(mut g) = store().lock() else {
        return;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return;
    };
    entry.release_sent_ms = now_ms;
    entry.next_attempt_ms = now_ms.saturating_add(RELEASE_BEFORE_WELCOME_MS);
}

/// True when welcome should wait for post-release backoff.
#[must_use]
pub fn defer_welcome_for_release(sniffer_id: u32, now_ms: u64) -> bool {
    if sniffer_id == 0 {
        return false;
    }
    let Ok(mut g) = store().lock() else {
        return false;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return false;
    };
    if !entry.release_before_welcome || entry.confirmed || entry.release_sent_ms == 0 {
        return false;
    }
    if now_ms.saturating_sub(entry.release_sent_ms) < RELEASE_BEFORE_WELCOME_MS {
        return true;
    }
    entry.release_before_welcome = false;
    entry.release_sent_ms = 0;
    false
}

/// Entries due for another welcome attempt (recent Hello first, capped per tick).
#[must_use]
pub fn due_attempts(now_ms: u64) -> Vec<(u32, [u8; 6], u8)> {
    due_attempts_capped(now_ms, MAX_WELCOME_ATTEMPTS_PER_TICK)
}

/// Like [`due_attempts`] but respects a caller-provided per-tick budget.
#[must_use]
pub fn due_attempts_capped(now_ms: u64, max: usize) -> Vec<(u32, [u8; 6], u8)> {
    if max == 0 {
        return Vec::new();
    }
    let Ok(g) = store().lock() else {
        return Vec::new();
    };
    let mut due: Vec<(u32, [u8; 6], u8, u64)> = g
        .pending
        .values()
        .filter(|e| {
            !e.confirmed
                && !e.timed_out
                && now_ms >= e.next_attempt_ms
                && now_ms.saturating_sub(e.armed_at_ms) < ADOPT_RETRY_DEADLINE_MS
        })
        .map(|e| (e.sniffer_id, e.mac, e.fleet_slot, e.last_hello_ms))
        .collect();
    due.sort_by(|a, b| b.3.cmp(&a.3));
    due.into_iter()
        .take(max)
        .map(|(id, mac, slot, _)| (id, mac, slot))
        .collect()
}

/// Record one complete welcome-bundle outcome and schedule its next retry.
///
/// A Start/Stop sequence is latched even if a later optional policy-list stage
/// failed, so its valid acknowledgement can still confirm adoption.
pub fn record_welcome_outcome(sniffer_id: u32, outcome: WelcomeAttemptResult, now_ms: u64) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(mut g) = store().lock() else {
        return;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return;
    };
    if entry.confirmed || entry.timed_out {
        return;
    }
    if entry.expected_ack_cmd_seq.is_none() {
        entry.expected_ack_cmd_seq = outcome.ack_cmd_seq;
    }
    entry.last_send_failed = !outcome.ok;
    let delay = backoff_ms(entry.attempt_count);
    entry.attempt_count = entry.attempt_count.saturating_add(1);
    entry.next_attempt_ms = now_ms.saturating_add(delay);
}

/// Record a successful welcome+start/stop transmit; wait for matching ack.
pub fn record_attempt(sniffer_id: u32, ack_cmd_seq: u32, now_ms: u64) {
    record_welcome_outcome(
        sniffer_id,
        WelcomeAttemptResult::success(ack_cmd_seq),
        now_ms,
    );
}

/// Welcome or Start/Stop synchronous send failed; schedule retry.
pub fn record_send_failure(sniffer_id: u32, now_ms: u64) {
    record_welcome_outcome(
        sniffer_id,
        WelcomeAttemptResult::failure(
            wardriver_fleet_realtime::welcome_bundle::WelcomeStage::EnsurePeer,
        ),
        now_ms,
    );
}

/// True when `ack_cmd_seq` matches the pending welcome attempt.
#[must_use]
pub fn try_confirm(sniffer_id: u32, peer_mac: &[u8; 6], ack_cmd_seq: u32) -> bool {
    if sniffer_id == 0 {
        return false;
    }
    let Ok(mut g) = store().lock() else {
        return false;
    };
    let Some(entry) = g.pending.get_mut(&sniffer_id) else {
        return false;
    };
    if entry.confirmed {
        return entry.mac == *peer_mac;
    }
    if entry.timed_out {
        return false;
    }
    let Some(expected) = entry.expected_ack_cmd_seq else {
        return false;
    };
    if entry.mac != *peer_mac || !ack_seq_matches(expected, ack_cmd_seq) {
        return false;
    }
    entry.confirmed = true;
    entry.expected_ack_cmd_seq = None;
    true
}

#[must_use]
pub fn is_confirmed(sniffer_id: u32) -> bool {
    store()
        .lock()
        .ok()
        .and_then(|g| g.pending.get(&sniffer_id).map(|e| e.confirmed))
        .unwrap_or(false)
}

/// Mark timed-out entries; returns sniffer ids newly timed out.
pub fn tick_timeouts(now_ms: u64) -> Vec<u32> {
    let Ok(mut g) = store().lock() else {
        return Vec::new();
    };
    let mut newly = Vec::new();
    for entry in g.pending.values_mut() {
        if entry.confirmed || entry.timed_out {
            continue;
        }
        if now_ms.saturating_sub(entry.armed_at_ms) < ADOPT_RETRY_DEADLINE_MS {
            continue;
        }
        entry.timed_out = true;
        entry.expected_ack_cmd_seq = None;
        entry.timed_out_hello_live = entry.last_hello_ms > 0
            && now_ms.saturating_sub(entry.last_hello_ms) < ADOPT_HELLO_LIVE_MS;
        newly.push(entry.sniffer_id);
    }
    newly
}

#[must_use]
pub fn adopt_status(sniffer_id: u32, now_ms: u64) -> (AdoptProgress, AdoptReason) {
    let Ok(g) = store().lock() else {
        return (AdoptProgress::None, AdoptReason::None);
    };
    let Some(entry) = g.pending.get(&sniffer_id) else {
        return (AdoptProgress::None, AdoptReason::None);
    };
    (entry.progress(now_ms), entry.reason())
}

/// Pending adoption rows for PackLink status when no stats slot exists yet.
#[must_use]
pub fn pending_status_rows(now_ms: u64) -> Vec<(u32, [u8; 6], u8, AdoptProgress, AdoptReason)> {
    let Ok(g) = store().lock() else {
        return Vec::new();
    };
    g.pending
        .values()
        .filter(|e| !e.confirmed)
        .map(|e| {
            (
                e.sniffer_id,
                e.mac,
                e.fleet_slot,
                e.progress(now_ms),
                e.reason(),
            )
        })
        .collect()
}

/// Orphan ack: rostered sniffer ack with no recorded welcome attempt.
pub fn note_orphan_ack(sniffer_id: u32) {
    if sniffer_id == 0 {
        return;
    }
    let Ok(g) = store().lock() else {
        return;
    };
    let Some(entry) = g.pending.get(&sniffer_id) else {
        return;
    };
    if entry.confirmed || entry.timed_out || entry.expected_ack_cmd_seq.is_some() {
        return;
    }
    // Diagnostic only — adoption FSM unchanged until a welcome attempt is recorded.
    let _ = entry.attempt_count;
}

/// True when `ack` is the expected Start/Stop seq or a wrap-safe newer control seq
/// (e.g. fleet Ping advanced `last_cmd_seq` before the sniffer flushed `SnifferAck`).
#[must_use]
pub fn ack_seq_matches(expected: u32, ack: u32) -> bool {
    ack.wrapping_sub(expected) < 0x8000_0000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        if let Ok(mut g) = store().lock() {
            g.pending.clear();
        }
    }

    #[test]
    fn rearm_confirmed_identity_change_resets_fsm() {
        reset();
        let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        let mac2 = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        arm(5, mac, 1, 1_000);
        record_attempt(5, 10, 1_000);
        assert!(try_confirm(5, &mac, 10));
        assert!(is_confirmed(5));
        arm(5, mac2, 2, 2_000);
        assert!(!is_confirmed(5));
        let due = due_attempts(2_000);
        assert!(due
            .iter()
            .any(|(id, m, slot)| *id == 5 && *m == mac2 && *slot == 2));
    }

    #[test]
    fn confirm_accepts_newer_cmd_seq_after_ping() {
        reset();
        let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        arm(7, mac, 2, 1_000);
        record_attempt(7, 42, 1_000);
        assert!(!try_confirm(7, &mac, 41));
        assert!(try_confirm(7, &mac, 45));
        assert!(is_confirmed(7));
    }

    #[test]
    fn confirm_requires_matching_cmd_seq() {
        reset();
        let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
        arm(9, mac, 2, 1_000);
        record_attempt(9, 42, 1_000);
        assert!(!try_confirm(9, &mac, 41));
        assert!(try_confirm(9, &mac, 42));
        assert!(is_confirmed(9));
    }

    #[test]
    fn retry_backoff_and_timeout_with_live_hello() {
        reset();
        let mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x01];
        let start = 10_000;
        arm(3, mac, 1, start);
        note_hello(3, start + ADOPT_RETRY_DEADLINE_MS - 2_000);
        record_send_failure(3, start + 250);
        let due = due_attempts(start + 750);
        assert!(due.iter().any(|(id, _, _)| *id == 3));
        tick_timeouts(start + ADOPT_RETRY_DEADLINE_MS);
        let (progress, reason) = adopt_status(3, start + ADOPT_RETRY_DEADLINE_MS);
        assert_eq!(progress, AdoptProgress::TimedOutHelloLive);
        assert_eq!(reason, AdoptReason::PossibleExistingBinding);
    }

    #[test]
    fn rearm_after_timeout_allows_retry() {
        reset();
        let mac = [0x02, 0x01, 0x02, 0x03, 0x04, 0x05];
        let start = 0;
        arm(9, mac, 4, start);
        tick_timeouts(start + ADOPT_RETRY_DEADLINE_MS);
        note_hello(9, start + ADOPT_RETRY_DEADLINE_MS + 100);
        let (progress, _) = adopt_status(9, start + ADOPT_RETRY_DEADLINE_MS + 100);
        assert_eq!(progress, AdoptProgress::RetryingWelcome);
    }
}
