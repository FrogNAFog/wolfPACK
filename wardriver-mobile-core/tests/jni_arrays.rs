//! Host tests for binary JNI array helpers.

use wardriver_mobile_core::jni_arrays::{
    cluster_map_markers_arrays, coverage_grid_arrays, gps_timeline_append, gps_timeline_create,
    gps_timeline_destroy, gps_timeline_to_hist_json, simplify_route_arrays,
};

#[test]
fn gps_timeline_handle_append_and_hist_json() {
    let handle = gps_timeline_create();
    assert!(handle > 0);
    let now = 1_700_000_000_000i64;
    assert!(gps_timeline_append(
        handle,
        now - 1_000,
        37.77,
        -122.42,
        8.0,
        None,
        now
    ));
    let json = gps_timeline_to_hist_json(handle);
    assert!(json.contains("\"gps_hist\""));
    assert!(json.contains("37.77"));
    gps_timeline_destroy(handle);
}

#[test]
fn simplify_route_arrays_reduces_point_count() {
    let lats: Vec<f64> = (0..100).map(|i| 37.0 + i as f64 * 0.0001).collect();
    let lons: Vec<f64> = (0..100).map(|i| -122.0 + i as f64 * 0.0001).collect();
    let (out_lats, _) = simplify_route_arrays(&lats, &lons, 10);
    assert!(out_lats.len() <= 10);
}

#[test]
fn cluster_map_markers_arrays_returns_member_indices() {
    let result = cluster_map_markers_arrays(
        &[37.0, 37.0],
        &[-122.0, -122.0],
        &[-50, -60],
        &["a".to_string(), "b".to_string()],
        &[
            "aa:bb:cc:dd:ee:01".to_string(),
            "aa:bb:cc:dd:ee:02".to_string(),
        ],
        &["WIFI".to_string(), "WIFI".to_string()],
        15.0,
        14.0,
        5,
    );
    assert_eq!(result.counts.len(), 1);
    assert_eq!(result.counts[0], 2);
    assert_eq!(result.member_indices[0].len(), 2);
}

#[test]
fn coverage_grid_arrays_fraction_and_unvisited() {
    let result = coverage_grid_arrays(37.0, 38.0, -123.0, -122.0, 4, 4, &[37.5], &[-122.5]);
    assert!(result.fraction > 0.0);
    assert!(result.fraction < 1.0);
    assert!(!result.unvisited_rows.is_empty());
    assert_eq!(result.unvisited_rows.len(), result.unvisited_cols.len());
}
