//! Host tests for map geometry (`wardriver_mobile_core::map_geometry`).

use wardriver_mobile_core::map_geometry::{
    cluster_map_markers, cluster_map_markers_indexed, simplify_route, ApMarker,
    CLUSTER_ZOOM_THRESHOLD, COORD_DECIMALS_DEFAULT, ROUTE_SIMPLIFY_THRESHOLD,
};

fn wifi_marker(lat: f64, lon: f64, ssid: &str, rssi: i32, bssid: &str) -> ApMarker {
    ApMarker {
        lat,
        lon,
        ssid: ssid.to_string(),
        rssi,
        bssid: bssid.to_string(),
        signal_kind: "WIFI".to_string(),
    }
}

#[test]
fn simplify_under_budget_returns_unchanged() {
    let pts = vec![(37.0, -122.0), (37.1, -122.1)];
    assert_eq!(simplify_route(&pts, ROUTE_SIMPLIFY_THRESHOLD), pts);
}

#[test]
fn simplify_reduces_large_route() {
    let pts: Vec<(f64, f64)> = (0..10_000)
        .map(|i| (37.0 + (i as f64) * 0.00001, -122.0))
        .collect();
    let out = simplify_route(&pts, 100);
    assert!(out.len() <= 100);
    assert_eq!(out.first(), Some(&(37.0, -122.0)));
}

#[test]
fn cluster_map_markers_stacks_coincident_coordinates() {
    let aps = vec![
        wifi_marker(37.12345, -122.54321, "A", -50, "aa:bb:cc:dd:ee:01"),
        wifi_marker(37.12345, -122.54321, "B", -40, "aa:bb:cc:dd:ee:02"),
    ];
    let clusters = cluster_map_markers(
        &aps,
        CLUSTER_ZOOM_THRESHOLD,
        COORD_DECIMALS_DEFAULT,
        CLUSTER_ZOOM_THRESHOLD,
    );
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].count, 2);
    assert_eq!(clusters[0].members.len(), 2);
}

#[test]
fn cluster_map_markers_grid_merge_concatenates_members() {
    let aps = vec![
        wifi_marker(37.0, -122.0, "A", -50, "aa:bb:cc:dd:ee:01"),
        wifi_marker(37.0001, -122.0001, "B", -40, "aa:bb:cc:dd:ee:02"),
        wifi_marker(37.0002, -122.0002, "C", -45, "aa:bb:cc:dd:ee:03"),
    ];
    let clusters = cluster_map_markers(&aps, 10.0, COORD_DECIMALS_DEFAULT, CLUSTER_ZOOM_THRESHOLD);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].members.len(), 3);
}

#[test]
fn cluster_map_markers_indexed_returns_input_indices() {
    let aps = vec![
        wifi_marker(37.12345, -122.54321, "A", -50, "aa:bb:cc:dd:ee:01"),
        wifi_marker(37.12345, -122.54321, "B", -40, "aa:bb:cc:dd:ee:02"),
    ];
    let clusters = cluster_map_markers_indexed(
        &aps,
        CLUSTER_ZOOM_THRESHOLD,
        COORD_DECIMALS_DEFAULT,
        CLUSTER_ZOOM_THRESHOLD,
    );
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].member_indices, vec![0, 1]);
}
