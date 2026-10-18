//! Host coverage for structured welcome bundle outcomes.

use wardriver_fleet_realtime::welcome_bundle::{
    WelcomeAttemptResult, WelcomeStage, WELCOME_STAGE_ORDER,
};

#[test]
fn welcome_stage_order_matches_bundle_stages() {
    assert_eq!(WELCOME_STAGE_ORDER.len(), 8);
    assert_eq!(WELCOME_STAGE_ORDER[0], WelcomeStage::EnsurePeer);
    assert_eq!(
        WELCOME_STAGE_ORDER.last().copied(),
        Some(WelcomeStage::SetRole)
    );
}

#[test]
fn failures_before_start_stop_have_no_ack_sequence() {
    let welcome_failure = WelcomeAttemptResult::failure(WelcomeStage::ControlWelcome);
    let start_stop_failure = WelcomeAttemptResult::failure(WelcomeStage::StartStop);

    assert_eq!(welcome_failure.ack_cmd_seq, None);
    assert_eq!(start_stop_failure.ack_cmd_seq, None);
}

#[test]
fn failures_after_start_stop_retain_ack_sequence() {
    let failure = WelcomeAttemptResult::failure_after_start_stop(WelcomeStage::SsidDenylist, 123);

    assert_eq!(failure.ack_cmd_seq, Some(123));
    assert!(!failure.ok);
}
