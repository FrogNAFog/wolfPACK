//! Host-testable BOOT long-press timing for fleet unbind (no esp-idf).

/// Continuous hold required to request fleet unbind on all roles.
pub const FLEET_UNBIND_HOLD_MS: u64 = 5_000;

/// True when `pressed_ms` meets the configured hold threshold.
#[must_use]
pub fn hold_complete(pressed_ms: u64, hold_ms: u64) -> bool {
    pressed_ms >= hold_ms
}

/// Whole seconds remaining before hold completes (`None` when complete).
#[must_use]
pub fn hold_seconds_remaining(pressed_ms: u64, hold_ms: u64) -> Option<u64> {
    if hold_complete(pressed_ms, hold_ms) {
        return None;
    }
    let rem = hold_ms.saturating_sub(pressed_ms);
    Some(rem.div_ceil(1000))
}
