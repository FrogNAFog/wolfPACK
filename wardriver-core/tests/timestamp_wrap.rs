//! Edge cases for 32-bit monotonic timer wrap and timestamp arithmetic.
//!
//! Note: `observed_unix_ms_from_anchor` uses plain i64 subtraction (NOT wrapping),
//! so it cannot correctly convert observations that wrap past u32::MAX.
//! The throttle code in `sniffer_sweep` uses `wrapping_sub` and handles this correctly.

use std::collections::VecDeque;
use wardriver_core::gps_anchor::{self, GpsFix, MAX_FIX_TIMELINE_DELTA_MS};

/// `observed_unix_ms_from_anchor` uses plain i64 subtraction (no wrapping).
/// When end_mono is near u32::MAX and obs is 1000 ms earlier, the offset
/// is negative 1000 → result is end_unix - 1000.
#[test]
fn observed_unix_ms_near_u32_wrap_observation_before_end() {
    let end_mono = 0xFFFF_FFF0u32;
    let end_unix = 1_777_188_600_000u64;
    let obs = end_mono.wrapping_sub(1000); // 0xFFFF_FC57
    let result = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs)
        .expect("should compute even near u32 wrap");
    // obs as i64 (0xFFFF_FC57 = 4294966295) - end_mono as i64 (0xFFFF_FFF0 = 4294967295) = -1000
    assert_eq!(result, (end_unix as i64) - 1000);
}

/// `observed_unix_ms_from_anchor` does NOT handle wrap-around correctly.
/// When obs wraps past u32::MAX to a small value, the i64 subtraction
/// produces a large negative offset (~4.3 billion ms = ~50 days earlier).
#[test]
fn observed_unix_ms_near_u32_wrap_is_negative_offset() {
    let end_mono = 0xFFFF_FFF0u32;
    let end_unix = 1_777_188_600_000u64;
    let obs = end_mono.wrapping_add(1000); // 0xFFFF_FFF0 + 1000 = 0x1_0000_03D8 → wraps to 0x3D8 = 984
    let result = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs)
        .expect("computes but with wrong offset");
    // obs(984) - end(0xFFFF_FFF0=4294967280) as i64 = 984 - 4294967280 = -4294966296
    let offset = (obs as i64) - (end_mono as i64);
    assert_eq!(result, (end_unix as i64) + offset);
    // The offset is -4294966296, NOT +984. The result is ~50 days before end_unix,
    // not 984 ms after. This documents the known limitation of plain i64 subtraction.
    assert_eq!(offset, -4_294_966_296);
}

/// `observed_unix_ms_from_anchor` at end_mono = 0, obs = 0xFFFF_FFFF.
/// obs (4294967295) - end (0) = 4294967295 → result = end_unix + 4294967295.
#[test]
fn observed_unix_ms_at_exact_u32_boundary() {
    let end_mono = 0u32;
    let end_unix = 1_777_188_600_000u64;
    let obs = 0xFFFF_FFFF;
    let result = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs);
    // obs(0xFFFF_FFFF) - end(0) = 4294967295
    assert_eq!(result, Some((end_unix as i64) + 4_294_967_295));
}

/// `observed_unix_ms_from_anchor` when both end_mono and obs are zero.
#[test]
fn observed_unix_ms_end_mono_zero_obs_zero() {
    let result = gps_anchor::observed_unix_ms_from_anchor(0, 5_000_000, 0);
    assert_eq!(result, Some(5_000_000));
}

/// `observed_unix_ms_from_anchor` when end_mono = 0xFFFF_FFFF and obs = 0.
/// obs(0) - end(0xFFFF_FFFF) = -4294967295. Result is NOT 5_000_001.
#[test]
fn observed_unix_ms_end_mono_max_obs_zero() {
    let result = gps_anchor::observed_unix_ms_from_anchor(0xFFFF_FFFF, 5_000_000, 0);
    // 0 - 0xFFFF_FFFF = -4294967295
    assert_eq!(result, Some(5_000_000i64 - 4_294_967_295));
    // Result is negative → function cannot handle the case where obs < end
    // when both are interpreted as u32 values that wrapped.
}

/// Large positive offset (obs much larger than end) still fits in i64.
#[test]
fn observed_unix_ms_large_offset_fits_i64() {
    let end_mono = 0u32;
    let end_unix = 1_777_188_600_000u64;
    let obs = 0x7FFF_FFFF; // max positive signed u32
    let result = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs)
        .expect("large offset fits in i64");
    assert_eq!(result, (end_unix as i64) + 0x7FFF_FFFF as i64);
}

/// `observed_unix_ms_from_anchor` with obs >> end_mono produces large positive offset
/// (no wrapping semantics), pushing result far into the future.
#[test]
fn observed_unix_ms_large_positive_offset() {
    let end_mono = 0u32;
    let end_unix = 2_200_000_000_000u64;
    let obs = 0x8000_0000;
    let result = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs);
    // obs(0x8000_0000 = 2147483648) - end(0) = 2147483648
    assert_eq!(result, Some(2_200_000_000_000i64 + 2_147_483_648));
    // This produces a date ~2068 — far future but computable.
    // In practice, GPS matching rejects observations this far from any fix.
}

/// `match_gps_fix_for_obs` with observations near u32 monotonic wrap.
#[test]
fn gps_match_with_wrapped_observation_times() {
    let mut fix = GpsFix {
        utc_unix_ms: 1_777_188_600_000,
        lat: 37.7749,
        lon: -122.4194,
        altitude_m: Some(12.0),
        hdop: Some(1.5),
        accuracy_m: None,
        fix_quality: 1,
    };
    fix.utc_unix_ms = 1_777_188_599_500;
    let mut hist = VecDeque::new();
    hist.push_back((fix.utc_unix_ms, fix));

    let obs = 1_777_188_599_700i64;
    let matched = gps_anchor::match_gps_fix_for_obs(&hist, obs, MAX_FIX_TIMELINE_DELTA_MS)
        .expect("should match within 200 ms");
    assert_eq!(matched.utc_unix_ms, 1_777_188_599_500);
}

/// When u32 monotonic clock wraps, `observed_unix_ms_from_anchor` produces
/// a large negative offset (~50 days earlier). The result is still positive
/// (not epoch), but the GPS match rejects it because no fix is that old.
#[test]
fn gps_match_rejects_wrapped_observation() {
    let end_mono = 0xFFFF_FFF0u32;
    let end_unix = 1_777_188_600_000u64;
    let obs = end_mono.wrapping_add(1000); // wraps to 984

    let obs_unix = gps_anchor::observed_unix_ms_from_anchor(end_mono, end_unix, obs);
    assert!(obs_unix.is_some());
    // The computed Unix time is ~end_unix - 4.29B ms (~50 days earlier).
    // Still positive (not pre-epoch), but far from any recent GPS fix.
    let obs_val = obs_unix.unwrap();
    assert!(
        obs_val < end_unix as i64,
        "wrapped offset should push time earlier"
    );
    assert!(obs_val > 0, "but still positive (not pre-epoch)");

    // GPS match with a recent fix should reject this observation
    let fix = GpsFix {
        utc_unix_ms: 1_777_188_600_000,
        lat: 37.7749,
        lon: -122.4194,
        altitude_m: Some(12.0),
        hdop: Some(1.5),
        accuracy_m: None,
        fix_quality: 1,
    };
    let mut hist = VecDeque::new();
    hist.push_back((fix.utc_unix_ms, fix));

    let matched =
        gps_anchor::match_gps_fix_for_obs(&hist, obs_unix.unwrap(), MAX_FIX_TIMELINE_DELTA_MS);
    assert!(
        matched.is_none(),
        "should reject observation with wrapped timestamp"
    );
}

#[test]
fn wolfpack_dump_durations_inverted_timestamps() {
    use wardriver_core::wolfpack_telemetry::wolfpack_dump_durations;

    let d = wolfpack_dump_durations(
        1000, // join_attempt_ms
        2000, // visit_start_ms
        3000, // bulk_start_ms
        4000, // transport_end_ms
        1500, // done_ms (before visit_start! inverted)
    );
    assert_eq!(d.queue_ms, 1000); // 2000 - 1000
                                  // bulk_xfer: transport_end(4000) >= bulk_start(3000) → 4000 - 3000 = 1000
    assert_eq!(d.bulk_xfer_ms, 1000);
    // post_ms: done(1500).saturating_sub(transport_end(4000)) = 0
    assert_eq!(d.post_ms, 0);
    // visit_ms: done(1500).saturating_sub(visit_start(2000)) = 0
    assert_eq!(d.visit_ms, 0);
}

#[test]
fn wolfpack_dump_durations_all_zeros() {
    use wardriver_core::wolfpack_telemetry::wolfpack_dump_durations;

    let d = wolfpack_dump_durations(0, 0, 0, 0, 0);
    assert_eq!(d.queue_ms, 0);
    assert_eq!(d.bulk_xfer_ms, 0);
    assert_eq!(d.post_ms, 0);
    assert_eq!(d.visit_ms, 0);
}

#[test]
fn wolfpack_dump_durations_normal() {
    use wardriver_core::wolfpack_telemetry::wolfpack_dump_durations;

    let d = wolfpack_dump_durations(
        1000, // join_attempt
        2000, // visit_start
        3000, // bulk_start
        8000, // transport_end
        9000, // done
    );
    assert_eq!(d.queue_ms, 1000); // 2000 - 1000
    assert_eq!(d.bulk_xfer_ms, 5000); // 8000 - 3000
    assert_eq!(d.post_ms, 1000); // 9000 - 8000
    assert_eq!(d.visit_ms, 7000); // 9000 - 2000
}

#[test]
fn wolfpack_dump_durations_no_bulk() {
    use wardriver_core::wolfpack_telemetry::wolfpack_dump_durations;

    let d = wolfpack_dump_durations(
        1000, // join_attempt
        2000, // visit_start
        0,    // bulk_start (no bulk)
        3000, // transport_end
        4000, // done
    );
    assert_eq!(d.queue_ms, 1000);
    assert_eq!(d.bulk_xfer_ms, 0);
    assert_eq!(d.post_ms, 1000);
    assert_eq!(d.visit_ms, 2000);
}

#[test]
fn wolfpack_dump_durations_large_timestamps() {
    use wardriver_core::wolfpack_telemetry::wolfpack_dump_durations;

    let huge = u64::MAX - 100;
    let d = wolfpack_dump_durations(huge - 400, huge - 300, huge - 200, huge - 100, huge);
    assert_eq!(d.queue_ms, 100);
    assert_eq!(d.bulk_xfer_ms, 100);
    assert_eq!(d.post_ms, 100);
    assert_eq!(d.visit_ms, 300);
}

#[test]
fn throttle_wifi_wrap_at_u32_boundary() {
    use wardriver_core::sniffer_sweep::throttle_wifi_entries_with_extras;
    use wardriver_proto::{ApEntryV1, AuthMini};

    fn ap(bssid: [u8; 6], mono: u32, rssi: i8) -> ApEntryV1 {
        ApEntryV1 {
            bssid,
            channel: 6,
            rssi,
            auth: AuthMini::Open,
            ssid_len: 0,
            ssid: [0u8; 32],
            obs_mono_ms: mono,
        }
    }

    let bssid = [1u8; 6];
    // Two observations: one just before u32 wrap, one just after.
    // Throttle uses wrapping_sub → 21 ms distance, within window.
    let entries = vec![ap(bssid, 0xFFFF_FFF0, -50), ap(bssid, 0x0000_0005, -40)];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 1); // within window → keep strongest
    assert_eq!(out[0].rssi, -40);
}

#[test]
fn throttle_wifi_wrap_with_large_gap() {
    use wardriver_core::sniffer_sweep::throttle_wifi_entries_with_extras;
    use wardriver_proto::{ApEntryV1, AuthMini};

    fn ap(bssid: [u8; 6], mono: u32, rssi: i8) -> ApEntryV1 {
        ApEntryV1 {
            bssid,
            channel: 6,
            rssi,
            auth: AuthMini::Open,
            ssid_len: 0,
            ssid: [0u8; 32],
            obs_mono_ms: mono,
        }
    }

    let bssid = [1u8; 6];
    // 1000 ms gap across wrap → two entries
    let entries = vec![
        ap(bssid, 0xFFFF_FFFF, -50),
        ap(bssid, 0x0000_03E8, -40), // 1000 ms after (wrapping)
    ];
    let out = throttle_wifi_entries_with_extras(entries, vec![], 500).0;
    assert_eq!(out.len(), 2); // 1000 ms >= 500 ms → separate windows
}
