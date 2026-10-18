//! Peer pool grace / eviction and refresh policy mirrors
//! [`wardriver_control::control::peer_pool`].
//!
//! These tests mirror policy logic for documentation. They do not exercise
//! production code — the actual peer_pool implementation is tested via
//! pipeline integration tests.
//! [`wardriver_control::control::peer_pool`].
//!
//! `peer_pool` is IDF-coupled; timing constants and eviction decisions are
//! duplicated here for host-only regression.

/// Must match `wardriver_control::control::peer_pool::POST_DUMP_GRACE_MS`.
const POST_DUMP_GRACE_MS: u64 = 650;
/// Must match `wardriver_control::control::peer_pool::POST_DUMP_DONE_GRACE_MS`.
const POST_DUMP_DONE_GRACE_MS: u64 = 100;
/// Must match `wardriver_control::control::peer_pool::PEER_REFRESH_GRACE_MS`.
const PEER_REFRESH_GRACE_MS: u64 = POST_DUMP_DONE_GRACE_MS;
/// Must match `wardriver_control::control::time_sync_rtt::TIME_SYNC_FALLBACK_MS`.
const TIME_SYNC_FALLBACK_MS: u64 = 250;
/// Must match `wardriver_core::peer_join_queue::VISIT_ARM_TIMEOUT_MS`.
const VISIT_ARM_TIMEOUT_MS: u64 = 5_000;

fn arm_post_dump_grace(now_ms: u64) -> u64 {
    now_ms.saturating_add(POST_DUMP_GRACE_MS)
}

fn shorten_grace_after_done(now_ms: u64) -> u64 {
    now_ms.saturating_add(POST_DUMP_DONE_GRACE_MS)
}

fn arm_peer_refresh_grace(now_ms: u64) -> u64 {
    now_ms.saturating_add(PEER_REFRESH_GRACE_MS)
}

/// Mirrors [`peer_pool::finish_ready`].
fn finish_ready(
    transport_end_ms: u64,
    flush_done: bool,
    done_sent: bool,
    time_sync_attempted: bool,
    time_sync_done: bool,
    now_ms: u64,
) -> bool {
    if transport_end_ms == 0 || !flush_done || done_sent {
        return false;
    }
    if !time_sync_attempted || time_sync_done {
        return true;
    }
    now_ms >= transport_end_ms.saturating_add(TIME_SYNC_FALLBACK_MS)
}

/// Mirrors [`tick_evict_pending`] due-MAC filter (one peer).
fn grace_evict_due(now_ms: u64, pending_evict_at: Option<u64>, has_incomplete: bool) -> bool {
    let Some(deadline) = pending_evict_at else {
        return false;
    };
    now_ms >= deadline && !has_incomplete
}

/// Mirrors [`lru_evictable`] visit-arm branch.
fn visit_arm_blocks(visit_armed_ms: Option<u64>, now_ms: u64) -> bool {
    visit_armed_ms.is_some_and(|t| now_ms < t.saturating_add(VISIT_ARM_TIMEOUT_MS))
}

/// Mirrors [`lru_evictable`] without pool lookup.
fn lru_evictable(
    in_exclude: bool,
    has_incomplete: bool,
    pending_evict_at: Option<u64>,
    visit_armed_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    !in_exclude
        && !has_incomplete
        && pending_evict_at.is_none()
        && !visit_arm_blocks(visit_armed_ms, now_ms)
}

#[test]
fn post_dump_grace_deadline_is_650ms() {
    assert_eq!(arm_post_dump_grace(1000), 1650);
    assert_eq!(arm_post_dump_grace(u64::MAX - 100), u64::MAX);
}

#[test]
fn post_dump_done_grace_is_100ms() {
    assert_eq!(shorten_grace_after_done(5000), 5100);
    assert_eq!(arm_peer_refresh_grace(5000), 5100);
}

#[test]
fn finish_ready_when_flush_done_and_rtt_skipped() {
    let t_end = 1000u64;
    assert!(finish_ready(t_end, true, false, false, true, t_end));
    assert!(!finish_ready(t_end, false, false, false, true, t_end));
    assert!(!finish_ready(0, true, false, false, true, t_end));
    assert!(!finish_ready(t_end, true, true, false, true, t_end));
}

#[test]
fn finish_ready_waits_for_rtt_when_attempted() {
    let t_end = 1000u64;
    assert!(!finish_ready(t_end, true, false, true, false, t_end));
    assert!(finish_ready(t_end, true, false, true, true, t_end));
    assert!(finish_ready(
        t_end,
        true,
        false,
        true,
        false,
        t_end + TIME_SYNC_FALLBACK_MS
    ));
}

#[test]
fn grace_evict_waits_until_deadline() {
    let deadline = arm_post_dump_grace(5000);
    assert!(!grace_evict_due(5000, Some(deadline), false));
    assert!(!grace_evict_due(deadline - 1, Some(deadline), false));
    assert!(grace_evict_due(deadline, Some(deadline), false));
    assert!(grace_evict_due(deadline + 1, Some(deadline), false));
}

#[test]
fn grace_evict_blocked_while_reasm_incomplete() {
    let deadline = arm_post_dump_grace(100);
    assert!(!grace_evict_due(deadline, Some(deadline), true));
    assert!(grace_evict_due(deadline, Some(deadline), false));
}

#[test]
fn grace_evict_requires_pending_evict_armed() {
    assert!(!grace_evict_due(10_000, None, false));
}

#[test]
fn lru_skips_grace_pending_incomplete_and_visit_arm() {
    assert!(lru_evictable(false, false, None, None, 0));
    assert!(!lru_evictable(false, true, None, None, 0));
    assert!(!lru_evictable(true, false, None, None, 0));
    assert!(!lru_evictable(false, false, Some(9999), None, 0));
    assert!(!lru_evictable(false, false, None, Some(1000), 1000));
    assert!(lru_evictable(
        false,
        false,
        None,
        Some(1000),
        1000 + VISIT_ARM_TIMEOUT_MS
    ));
}

use wardriver_proto::PeerJoinReason;

/// Mirrors [`peer_pool::PostDumpSession`] fields used for refresh gating.
#[derive(Clone, Copy, Debug, Default)]
struct RefreshSession {
    join_reason: u8,
    visit_start_ms: u64,
    bulk_start_ms: u64,
    transport_end_ms: u64,
    flush_done: bool,
    done_sent: bool,
}

/// Mirrors [`peer_pool::is_active_peer_refresh`].
fn is_active_peer_refresh(s: &RefreshSession) -> bool {
    s.join_reason == PeerJoinReason::PeerRefresh as u8
        && s.visit_start_ms != 0
        && s.bulk_start_ms == 0
        && s.transport_end_ms == 0
}

/// Mirrors [`peer_pool::session_pending_post_dump_finish`].
fn session_pending_post_dump_finish(s: &RefreshSession) -> bool {
    s.transport_end_ms != 0
        && s.flush_done
        && !s.done_sent
        && s.join_reason != PeerJoinReason::PeerRefresh as u8
}

/// Mirrors post-`complete_peer_refresh_visit` session state.
fn complete_peer_refresh_visit(s: &mut RefreshSession, now_ms: u64) {
    if !is_active_peer_refresh(s) {
        return;
    }
    if s.transport_end_ms == 0 {
        s.transport_end_ms = now_ms;
    }
    s.flush_done = true;
    s.done_sent = true;
}

// ── Peer refresh policy tests ──

#[test]
fn peer_refresh_grace_is_100ms() {
    assert_eq!(arm_peer_refresh_grace(5000), 5100);
}

#[test]
fn active_peer_refresh_requires_admitted_zero_bulk_visit() {
    let mut s = RefreshSession {
        join_reason: PeerJoinReason::PeerRefresh as u8,
        visit_start_ms: 1000,
        ..RefreshSession::default()
    };
    assert!(is_active_peer_refresh(&s));

    s.join_reason = PeerJoinReason::DumpWifi as u8;
    assert!(!is_active_peer_refresh(&s));

    s.join_reason = PeerJoinReason::PeerRefresh as u8;
    s.bulk_start_ms = 1001;
    assert!(!is_active_peer_refresh(&s));

    s.bulk_start_ms = 0;
    s.transport_end_ms = 1002;
    assert!(!is_active_peer_refresh(&s));

    s.transport_end_ms = 0;
    s.visit_start_ms = 0;
    assert!(!is_active_peer_refresh(&s));
}

#[test]
fn completed_peer_refresh_not_pending_post_dump_finish() {
    let mut s = RefreshSession {
        join_reason: PeerJoinReason::PeerRefresh as u8,
        visit_start_ms: 1000,
        ..RefreshSession::default()
    };
    complete_peer_refresh_visit(&mut s, 2000);
    assert!(!session_pending_post_dump_finish(&s));
    assert!(s.done_sent);
    assert!(s.flush_done);
}

#[test]
fn dump_visit_still_pending_finish_when_flush_done() {
    let s = RefreshSession {
        join_reason: PeerJoinReason::DumpWifi as u8,
        visit_start_ms: 1000,
        transport_end_ms: 1500,
        flush_done: true,
        done_sent: false,
        ..RefreshSession::default()
    };
    assert!(session_pending_post_dump_finish(&s));
}

#[test]
fn peer_refresh_excluded_even_if_done_sent_regressed() {
    let s = RefreshSession {
        join_reason: PeerJoinReason::PeerRefresh as u8,
        visit_start_ms: 1000,
        transport_end_ms: 2000,
        flush_done: true,
        done_sent: false,
        ..RefreshSession::default()
    };
    assert!(!session_pending_post_dump_finish(&s));
}
