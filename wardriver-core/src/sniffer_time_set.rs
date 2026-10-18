//! Host-testable sniffer wall-clock apply decision (no esp-idf).
//!
//! The pure part of `wardriver-sniffer/src/sniffer_espnow.rs`'s `apply_time_set`: the
//! epoch-plausibility gate and the `settimeofday` return-code mapping. The sniffer keeps the
//! `sys::settimeofday` syscall and hands its `rc` to [`decide_time_set_apply`].

/// Result of attempting to apply a control-offered wall clock to the sniffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSetApplyOutcome {
    /// `settimeofday` succeeded with a plausible epoch.
    Applied,
    /// Offered epoch below minimum (1970 / boot garbage) — intentionally ignored.
    RejectedEpoch,
    /// Plausible epoch but `settimeofday` returned a non-zero rc.
    SettimeofdayFailed(i32),
}

/// Pure decision for a control-offered wall clock.
///
/// Mirrors `apply_time_set` in the sniffer: reject epochs below
/// [`wardriver_proto::TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS`] before the syscall is even considered;
/// otherwise map the `settimeofday` return code to [`TimeSetApplyOutcome::SettimeofdayFailed`]
/// (non-zero rc) or [`TimeSetApplyOutcome::Applied`] (zero rc).
pub fn decide_time_set_apply(unix_ms: u64, settimeofday_rc: i32) -> TimeSetApplyOutcome {
    if unix_ms < wardriver_proto::TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS {
        return TimeSetApplyOutcome::RejectedEpoch;
    }
    if settimeofday_rc != 0 {
        return TimeSetApplyOutcome::SettimeofdayFailed(settimeofday_rc);
    }
    TimeSetApplyOutcome::Applied
}
