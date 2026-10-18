//! Host tests for probe-burst q8 average bookkeeping (`wardriver_core::probe_burst`).
//!
//! Covers the q8 rounding, the i16 clamp at both bounds, and the n = max(count, 1) floor.

use wardriver_core::probe_burst::rssi_avg_q8;

#[test]
fn single_sample_q8() {
    // One sample of -40 dBm -> -40 * 256 = -10240.
    assert_eq!(rssi_avg_q8(-40, 1), -10_240);
}

#[test]
fn multi_sample_average_q8_rounding() {
    // Sum -100 over 3 samples: (-100 * 256) / 3 = -25600 / 3 = -8533 (truncates toward zero).
    assert_eq!(rssi_avg_q8(-100, 3), -8_533);
    // Sum -101 over 3: -25856 / 3 = -8618 (truncates toward zero).
    assert_eq!(rssi_avg_q8(-101, 3), -8_618);
}

#[test]
fn zero_count_uses_one() {
    // count = 0 must behave like count = 1 (n = max(count, 1)).
    assert_eq!(rssi_avg_q8(-40, 0), -10_240);
    assert_eq!(rssi_avg_q8(0, 0), 0);
}

#[test]
fn clamps_to_i16_bounds() {
    // Large positive sum clamps to i16::MAX.
    assert_eq!(rssi_avg_q8(i32::MAX, 1), i16::MAX);
    assert_eq!(rssi_avg_q8(300_000, 1), i16::MAX);
    // Large negative sum clamps to i16::MIN.
    assert_eq!(rssi_avg_q8(i32::MIN, 1), i16::MIN);
    assert_eq!(rssi_avg_q8(-300_000, 1), i16::MIN);
}

#[test]
fn saturating_mul_does_not_panic() {
    // i32::MIN * 256 saturates rather than overflowing.
    assert_eq!(rssi_avg_q8(i32::MIN, 1), i16::MIN);
    // i32::MAX * 256 saturates rather than overflowing.
    assert_eq!(rssi_avg_q8(i32::MAX, 1), i16::MAX);
}
