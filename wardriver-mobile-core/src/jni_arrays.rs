//! Binary JNI helpers for map geometry, coverage grid, and GPS timeline handles.
//!
//! Host-testable logic lives here; [`crate::jni`] exposes thin JNI wrappers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::gps_timeline::GpsTimeline;
use crate::map_geometry::{
    cluster_map_markers_indexed, simplify_route, ApMarker, CLUSTER_ZOOM_THRESHOLD,
    COORD_DECIMALS_DEFAULT, ROUTE_SIMPLIFY_THRESHOLD,
};
use serde_json::{json, Value};

use crate::coverage_grid::CoverageGrid;
use crate::geofence::GeofenceSquare;

static TIMELINE_STORE: OnceLock<Mutex<HashMap<i64, GpsTimeline>>> = OnceLock::new();
static NEXT_TIMELINE_HANDLE: AtomicI64 = AtomicI64::new(1);

fn timeline_store() -> &'static Mutex<HashMap<i64, GpsTimeline>> {
    TIMELINE_STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Allocate a native GPS timeline; returns 0 on failure.
#[must_use]
pub fn gps_timeline_create() -> i64 {
    let handle = NEXT_TIMELINE_HANDLE.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut store) = timeline_store().lock() {
        store.insert(handle, GpsTimeline::new());
        handle
    } else {
        0
    }
}

/// Drop a native GPS timeline handle (idempotent).
pub fn gps_timeline_destroy(handle: i64) {
    if handle == 0 {
        return;
    }
    if let Ok(mut store) = timeline_store().lock() {
        store.remove(&handle);
    }
}

/// Append one fix to a native timeline.
#[must_use]
pub fn gps_timeline_append(
    handle: i64,
    unix_ms: i64,
    lat: f64,
    lon: f64,
    accuracy_m: f32,
    altitude_m: Option<f64>,
    now_ms: i64,
) -> bool {
    let Ok(mut store) = timeline_store().lock() else {
        return false;
    };
    let Some(timeline) = store.get_mut(&handle) else {
        return false;
    };
    timeline.append(unix_ms, lat, lon, accuracy_m, altitude_m, now_ms)
}

/// Prune samples older than retention window.
pub fn gps_timeline_prune(handle: i64, now_ms: i64) {
    if let Ok(mut store) = timeline_store().lock() {
        if let Some(timeline) = store.get_mut(&handle) {
            timeline.prune(now_ms);
        }
    }
}

/// Merge route points into a native timeline.
pub fn gps_timeline_merge_route(
    handle: i64,
    unix_ms: &[i64],
    lats: &[f64],
    lons: &[f64],
    accuracy_m: &[f32],
    now_ms: i64,
) {
    let Ok(mut store) = timeline_store().lock() else {
        return;
    };
    let Some(timeline) = store.get_mut(&handle) else {
        return;
    };
    let points: Vec<(i64, f64, f64, f32, Option<f64>)> = unix_ms
        .iter()
        .zip(lats.iter())
        .zip(lons.iter())
        .zip(accuracy_m.iter())
        .map(|(((u, lat), lon), acc)| (*u, *lat, *lon, *acc, None))
        .collect();
    timeline.merge_samples(points, now_ms);
}

/// JSON snapshot of timeline samples for ingest `gps_hist` (called only at bulk ingest).
#[must_use]
pub fn gps_timeline_to_hist_json(handle: i64) -> String {
    let Ok(store) = timeline_store().lock() else {
        return json!({"ok": false, "error": "lock failed"}).to_string();
    };
    let Some(timeline) = store.get(&handle) else {
        return json!({"ok": false, "error": "invalid handle"}).to_string();
    };
    let samples: Vec<Value> = timeline
        .to_fix_hist()
        .iter()
        .map(|(_, fix)| {
            let mut obj = json!({
                "unix_ms": fix.utc_unix_ms,
                "lat": fix.lat,
                "lon": fix.lon,
                "accuracy_m": fix.accuracy_m,
            });
            if let Some(alt) = fix.altitude_m {
                obj["altitude_m"] = json!(alt);
            }
            obj
        })
        .collect();
    json!({
        "ok": true,
        "gps_usable": timeline.latest_usable(),
        "size": timeline.len(),
        "gps_hist": samples,
    })
    .to_string()
}

/// Douglas-Peucker route simplification over parallel lat/lon arrays.
#[must_use]
pub fn simplify_route_arrays(
    lats: &[f64],
    lons: &[f64],
    max_points: usize,
) -> (Vec<f64>, Vec<f64>) {
    let points: Vec<(f64, f64)> = lats
        .iter()
        .zip(lons)
        .map(|(&lat, &lon)| (lat, lon))
        .collect();
    let out = simplify_route(&points, max_points);
    let (olats, olons): (Vec<_>, Vec<_>) = out.into_iter().unzip();
    (olats, olons)
}

/// Cluster result with member indices into the input marker slice.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterArraysResult {
    pub lats: Vec<f64>,
    pub lons: Vec<f64>,
    pub counts: Vec<i32>,
    pub best_rssi: Vec<i32>,
    pub label_ssids: Vec<String>,
    pub member_indices: Vec<Vec<i32>>,
}

/// Grid/stack clustering without round-tripping marker strings through JSON.
#[must_use]
pub fn cluster_map_markers_arrays(
    lats: &[f64],
    lons: &[f64],
    rssis: &[i32],
    ssids: &[String],
    bssids: &[String],
    signal_kinds: &[String],
    zoom: f64,
    cluster_zoom_threshold: f64,
    coord_decimals: u32,
) -> ClusterArraysResult {
    let len = lats.len();
    let aps: Vec<ApMarker> = (0..len)
        .map(|i| ApMarker {
            lat: lats[i],
            lon: lons[i],
            ssid: ssids.get(i).cloned().unwrap_or_default(),
            rssi: rssis.get(i).copied().unwrap_or(0),
            bssid: bssids.get(i).cloned().unwrap_or_default(),
            signal_kind: signal_kinds
                .get(i)
                .cloned()
                .unwrap_or_else(|| "WIFI".to_string()),
        })
        .collect();
    let clusters = cluster_map_markers_indexed(&aps, zoom, coord_decimals, cluster_zoom_threshold);
    let mut out = ClusterArraysResult {
        lats: Vec::with_capacity(clusters.len()),
        lons: Vec::with_capacity(clusters.len()),
        counts: Vec::with_capacity(clusters.len()),
        best_rssi: Vec::with_capacity(clusters.len()),
        label_ssids: Vec::with_capacity(clusters.len()),
        member_indices: Vec::with_capacity(clusters.len()),
    };
    for c in clusters {
        out.lats.push(c.lat);
        out.lons.push(c.lon);
        out.counts.push(c.count as i32);
        out.best_rssi.push(c.best_rssi);
        out.label_ssids.push(c.label_ssid);
        out.member_indices
            .push(c.member_indices.into_iter().map(|i| i as i32).collect());
    }
    out
}

/// Coverage grid fraction and unvisited cell coordinates in one pass.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverageArraysResult {
    pub fraction: f64,
    pub unvisited_rows: Vec<i32>,
    pub unvisited_cols: Vec<i32>,
}

#[must_use]
pub fn coverage_grid_arrays(
    lat_min: f64,
    lat_max: f64,
    lon_min: f64,
    lon_max: f64,
    rows: u32,
    cols: u32,
    route_lats: &[f64],
    route_lons: &[f64],
) -> CoverageArraysResult {
    let square = GeofenceSquare {
        lat_min,
        lat_max,
        lon_min,
        lon_max,
    };
    let mut grid = CoverageGrid::new(square, rows, cols);
    for (&lat, &lon) in route_lats.iter().zip(route_lons) {
        grid.visit(lat, lon);
    }
    let unvisited = grid.unvisited_cells();
    let mut rows_out = Vec::with_capacity(unvisited.len());
    let mut cols_out = Vec::with_capacity(unvisited.len());
    for cell in unvisited {
        rows_out.push(cell.row as i32);
        cols_out.push(cell.col as i32);
    }
    CoverageArraysResult {
        fraction: grid.coverage_fraction(),
        unvisited_rows: rows_out,
        unvisited_cols: cols_out,
    }
}

/// Default max points for route simplification.
#[must_use]
pub const fn default_simplify_max_points() -> usize {
    ROUTE_SIMPLIFY_THRESHOLD
}

/// Default cluster zoom threshold.
#[must_use]
pub const fn default_cluster_zoom_threshold() -> f64 {
    CLUSTER_ZOOM_THRESHOLD
}

/// Default coordinate decimals for stacking.
#[must_use]
pub const fn default_coord_decimals() -> u32 {
    COORD_DECIMALS_DEFAULT
}
