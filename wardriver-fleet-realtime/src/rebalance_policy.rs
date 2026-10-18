//! Pure policy for deciding whether a dedicated channel plan needs another push.

/// True when the current plan or active Wi-Fi set has not yet been dispatched.
///
/// Plan revisions can push immediately. Active-set changes wait for the minimum
/// rebalance interval; leaving the last-pushed signature unchanged keeps a
/// deferred change eligible on a later tick.
#[must_use]
pub fn should_rebalance(
    last_pushed_revision: Option<u32>,
    last_pushed_active_sig: Option<u64>,
    current_revision: u32,
    current_active_sig: u64,
    elapsed_since_push_ms: u64,
    min_interval_ms: u64,
) -> bool {
    if last_pushed_revision != Some(current_revision) {
        return true;
    }
    last_pushed_active_sig != Some(current_active_sig) && elapsed_since_push_ms >= min_interval_ms
}
