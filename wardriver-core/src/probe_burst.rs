//! Host-testable probe-burst q8 average bookkeeping (no esp-idf).
//!
//! Pure arithmetic extracted from `wardriver-sniffer/src/wifi_promisc.rs`
//! (`ProbeReqBurstAccum::to_sample` / `into_burst`): the q8 RSSI average with saturation and
//! i16 clamping.

/// Average of a saturated q8 RSSI sum over `count` observations, clamped to `i16`.
///
/// `count` is the wildcard probe count; a zero count is treated as one to avoid division by
/// zero. Mirrors the sniffer exactly: `(rssi_sum * 256) / max(count, 1)` then clamp to the
/// `i16` range.
pub fn rssi_avg_q8(rssi_sum: i32, count: u16) -> i16 {
    let n = (count as i32).max(1);
    let avg_q8_i32 = (rssi_sum.saturating_mul(256)) / n;
    avg_q8_i32.clamp(i16::MIN as i32, i16::MAX as i32) as i16
}
