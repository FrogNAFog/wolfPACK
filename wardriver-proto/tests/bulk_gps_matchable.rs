use wardriver_proto::{bulk_gps_matchable, GPS_HIST_RETENTION_MS};

#[test]
fn bulk_gps_matchable_rejects_zero_scan_end() {
    assert!(!bulk_gps_matchable(0, 1_000_000));
}

#[test]
fn bulk_gps_matchable_within_retention() {
    let now = 1_000_000_u64;
    let scan_end = now - GPS_HIST_RETENTION_MS / 2;
    assert!(bulk_gps_matchable(scan_end, now));
}

#[test]
fn bulk_gps_matchable_at_retention_boundary() {
    let now = 1_000_000_u64;
    let scan_end = now - GPS_HIST_RETENTION_MS;
    assert!(bulk_gps_matchable(scan_end, now));
}

#[test]
fn bulk_gps_matchable_past_retention() {
    let now = 1_000_000_u64;
    let scan_end = now - GPS_HIST_RETENTION_MS - 1;
    assert!(!bulk_gps_matchable(scan_end, now));
}

#[test]
fn bulk_gps_matchable_future_scan_end() {
    let now = 1_000_000_u64;
    let scan_end = now + 60_000;
    assert!(bulk_gps_matchable(scan_end, now));
}
