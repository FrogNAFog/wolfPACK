//! Host tests for gateway adoption retry FSM.

use wardriver_fleet_realtime::welcome_bundle::{WelcomeAttemptResult, WelcomeStage};
use wardriver_gateway::gateway_adoption::{
    ack_seq_matches, adopt_status, arm, cancel, due_attempts, is_confirmed, needs_release_send,
    note_hello, record_attempt, record_send_failure, record_welcome_outcome, tick_timeouts,
    try_confirm, AdoptProgress, AdoptReason, ADOPT_RETRY_DEADLINE_MS,
};

#[test]
fn stale_ack_ignored() {
    let mac = [0x02, 0x01, 0x02, 0x03, 0x04, 0x05];
    arm(4, mac, 1, 1_000);
    record_attempt(4, 100, 1_000);
    assert!(!try_confirm(4, &mac, 99));
    assert!(!is_confirmed(4));
}

#[test]
fn newer_ack_after_ping_confirms() {
    let mac = [0x02, 0x01, 0x02, 0x03, 0x04, 0x06];
    arm(5, mac, 2, 2_000);
    record_attempt(5, 42, 2_000);
    assert!(try_confirm(5, &mac, 44));
    assert!(is_confirmed(5));
}

#[test]
fn duplicate_ack_after_confirm_is_safe() {
    let mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    arm(8, mac, 3, 0);
    record_attempt(8, 7, 0);
    assert!(try_confirm(8, &mac, 7));
    assert!(try_confirm(8, &mac, 7));
}

#[test]
fn roster_cancel_clears_pending() {
    let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    arm(12, mac, 2, 500);
    cancel(12);
    assert!(due_attempts(500).iter().all(|(id, _, _)| *id != 12));
    let (progress, _) = adopt_status(12, 500);
    assert_eq!(progress, AdoptProgress::None);
}

#[test]
fn rearm_after_timeout_allows_retry() {
    let mac = [0x02, 0xDE, 0xAD, 0xBE, 0xEF, 0x00];
    arm(20, mac, 1, 0);
    tick_timeouts(ADOPT_RETRY_DEADLINE_MS);
    let (progress, reason) = adopt_status(20, ADOPT_RETRY_DEADLINE_MS);
    assert_eq!(progress, AdoptProgress::TimedOutNoHello);
    assert_eq!(reason, AdoptReason::None);
    arm(20, mac, 1, ADOPT_RETRY_DEADLINE_MS + 1);
    let (progress, _) = adopt_status(20, ADOPT_RETRY_DEADLINE_MS + 1);
    assert_eq!(progress, AdoptProgress::WaitingPolicy);
}

#[test]
fn cmd_seq_wrap_accepts_forward_progress() {
    assert!(ack_seq_matches(u32::MAX, u32::MAX));
    assert!(ack_seq_matches(u32::MAX, 0));
    assert!(!ack_seq_matches(u32::MAX, u32::MAX - 1));
    assert!(!ack_seq_matches(100, 50));
    assert!(ack_seq_matches(100, 105));
}

#[test]
fn delayed_ack_for_earlier_attempt_confirms() {
    let mac = [0x02, 0x10, 0x20, 0x30, 0x40, 0x50];
    arm(1, mac, 1, 1_000);
    record_attempt(1, 100, 1_000);
    record_attempt(1, 105, 1_500);
    assert!(try_confirm(1, &mac, 100));
    assert!(is_confirmed(1));
}

#[test]
fn idempotent_policy_resync_preserves_outstanding_seq() {
    let mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    arm(2, mac, 3, 5_000);
    record_attempt(2, 77, 5_100);
    arm(2, mac, 3, 5_200);
    record_attempt(2, 80, 5_300);
    assert!(try_confirm(2, &mac, 77));
    assert!(is_confirmed(2));
}

#[test]
fn send_failure_preserves_outstanding_seq() {
    let mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x01];
    arm(6, mac, 2, 0);
    record_attempt(6, 42, 0);
    record_send_failure(6, 250);
    assert!(try_confirm(6, &mac, 42));
    assert!(is_confirmed(6));
}

#[test]
fn first_retry_backoff_is_250ms() {
    let mac = [0x02, 0x01, 0x02, 0x03, 0x04, 0x07];
    arm(7, mac, 1, 1_000);
    record_send_failure(7, 1_000);
    assert!(due_attempts(1_249).is_empty());
    assert!(due_attempts(1_250).iter().any(|(id, _, _)| *id == 7));
}

#[test]
fn multiple_sniffers_confirm_independently() {
    let mac_a = [0x02, 0x01, 0x02, 0x03, 0x04, 0x08];
    let mac_b = [0x02, 0x11, 0x22, 0x33, 0x44, 0x09];
    arm(10, mac_a, 1, 0);
    arm(11, mac_b, 2, 0);
    record_attempt(10, 20, 0);
    record_attempt(11, 30, 0);
    assert!(try_confirm(10, &mac_a, 20));
    assert!(!is_confirmed(11));
    assert!(try_confirm(11, &mac_b, 30));
}

#[test]
fn identity_change_resets_outstanding_seq() {
    let mac_a = [0x02, 0x01, 0x02, 0x03, 0x04, 0x0A];
    let mac_b = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0x0B];
    arm(15, mac_a, 1, 0);
    record_attempt(15, 50, 0);
    arm(15, mac_b, 1, 100);
    assert!(!try_confirm(15, &mac_b, 50));
    record_attempt(15, 60, 200);
    assert!(try_confirm(15, &mac_b, 60));
}

#[test]
fn first_partial_failure_latches_ack_and_increments_backoff_once() {
    let mac = [0x02, 0x21, 0x22, 0x23, 0x24, 0x25];
    arm(30, mac, 1, 10_000);
    record_welcome_outcome(
        30,
        WelcomeAttemptResult::failure_after_start_stop(WelcomeStage::BssidDenylist, 42),
        10_000,
    );

    assert!(due_attempts(10_249).iter().all(|(id, _, _)| *id != 30));
    assert!(due_attempts(10_250).iter().any(|(id, _, _)| *id == 30));
    assert!(try_confirm(30, &mac, 42));
    assert!(is_confirmed(30));
}

#[test]
fn later_welcome_outcome_preserves_earliest_ack_sequence() {
    let mac = [0x02, 0x31, 0x32, 0x33, 0x34, 0x35];
    arm(31, mac, 1, 20_000);
    record_welcome_outcome(
        31,
        WelcomeAttemptResult::failure_after_start_stop(WelcomeStage::SsidWatchlist, 50),
        20_000,
    );
    record_welcome_outcome(31, WelcomeAttemptResult::success(55), 20_250);

    assert!(try_confirm(31, &mac, 50));
    assert!(is_confirmed(31));
}

#[test]
fn live_hello_after_timeout_reopens_retries() {
    let mac = [0x02, 0xDE, 0xAD, 0xBE, 0xEF, 0x0C];
    arm(21, mac, 1, 0);
    tick_timeouts(ADOPT_RETRY_DEADLINE_MS);
    note_hello(21, ADOPT_RETRY_DEADLINE_MS + 100);
    let (progress, _) = adopt_status(21, ADOPT_RETRY_DEADLINE_MS + 100);
    assert_eq!(progress, AdoptProgress::RetryingWelcome);
    assert!(!needs_release_send(21));
}

#[test]
fn idempotent_arm_confirmed_no_release() {
    let mac = [0x02, 0x40, 0x41, 0x42, 0x43, 0x44];
    arm(40, mac, 2, 1_000);
    record_attempt(40, 10, 1_000);
    assert!(try_confirm(40, &mac, 10));
    assert!(is_confirmed(40));
    arm(40, mac, 2, 2_000);
    assert!(!needs_release_send(40));
    note_hello(40, 2_100);
    assert!(!needs_release_send(40));
}

#[test]
fn timeout_rearm_without_identity_skips_release() {
    let mac = [0x02, 0x50, 0x51, 0x52, 0x53, 0x54];
    arm(50, mac, 1, 0);
    assert!(needs_release_send(50));
    tick_timeouts(ADOPT_RETRY_DEADLINE_MS);
    arm(50, mac, 1, ADOPT_RETRY_DEADLINE_MS + 1);
    assert!(!needs_release_send(50));
}

#[test]
fn note_hello_after_timeout_no_release() {
    let mac = [0x02, 0x60, 0x61, 0x62, 0x63, 0x64];
    arm(60, mac, 3, 0);
    tick_timeouts(ADOPT_RETRY_DEADLINE_MS);
    note_hello(60, ADOPT_RETRY_DEADLINE_MS + 50);
    assert!(!needs_release_send(60));
}

#[test]
fn identity_change_sets_release() {
    let mac_a = [0x02, 0x70, 0x71, 0x72, 0x73, 0x74];
    let mac_b = [0x02, 0x80, 0x81, 0x82, 0x83, 0x84];
    arm(70, mac_a, 1, 0);
    record_attempt(70, 5, 0);
    assert!(try_confirm(70, &mac_a, 5));
    arm(70, mac_b, 1, 100);
    assert!(!is_confirmed(70));
    assert!(needs_release_send(70));
}
