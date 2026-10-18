//! Host tests for sniffer wall-clock apply decision (`wardriver_core::sniffer_time_set`).
//!
//! Covers the epoch-plausibility gate (RejectedEpoch) and the settimeofday rc mapping
//! (SettimeofdayFailed / Applied).

use wardriver_core::sniffer_time_set::{decide_time_set_apply, TimeSetApplyOutcome};
use wardriver_proto::TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS;

#[test]
fn below_min_plausible_rejected_epoch() {
    // 1970 / boot garbage, even with a zero rc, is rejected before the syscall result matters.
    assert_eq!(
        decide_time_set_apply(0, 0),
        TimeSetApplyOutcome::RejectedEpoch
    );
    assert_eq!(
        decide_time_set_apply(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS - 1, 0),
        TimeSetApplyOutcome::RejectedEpoch
    );
    // A non-zero rc is irrelevant when the epoch is already implausible.
    assert_eq!(
        decide_time_set_apply(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS - 1, -1),
        TimeSetApplyOutcome::RejectedEpoch
    );
}

#[test]
fn plausible_rc_zero_applied() {
    assert_eq!(
        decide_time_set_apply(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS, 0),
        TimeSetApplyOutcome::Applied
    );
    assert_eq!(
        decide_time_set_apply(1_777_190_400_000, 0),
        TimeSetApplyOutcome::Applied
    );
}

#[test]
fn plausible_nonzero_rc_settimeofday_failed() {
    assert_eq!(
        decide_time_set_apply(TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS, -1),
        TimeSetApplyOutcome::SettimeofdayFailed(-1)
    );
    assert_eq!(
        decide_time_set_apply(1_777_190_400_000, 17),
        TimeSetApplyOutcome::SettimeofdayFailed(17)
    );
}
