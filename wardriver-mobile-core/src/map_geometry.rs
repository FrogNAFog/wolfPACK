//! Map polyline simplification and AP marker clustering (OSMDroid helpers).

/// Route point budget before Douglas-Peucker kicks in.
pub const ROUTE_SIMPLIFY_THRESHOLD: usize = 5000;

const SIMPLIFY_EPSILON_DEG: f64 = 0.00003;

/// AP marker input for clustering.
#[derive(Clone, Debug, PartialEq)]
pub struct ApMarker {
    pub lat: f64,
    pub lon: f64,
    pub ssid: String,
    pub rssi: i32,
    pub bssid: String,
    pub signal_kind: String,
}

/// Clustered AP marker for map overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct ApCluster {
    pub lat: f64,
    pub lon: f64,
    pub count: u32,
    pub label_ssid: String,
    pub best_rssi: i32,
    pub members: Vec<ApMarker>,
}

pub const CLUSTER_ZOOM_THRESHOLD: f64 = 14.0;

/// Default coordinate decimal places for coincident-device stacking (5 dp).
pub const COORD_DECIMALS_DEFAULT: u32 = 5;

/// Douglas-Peucker polyline simplification when route exceeds point budget.
#[must_use]
pub fn simplify_route(points: &[(f64, f64)], max_points: usize) -> Vec<(f64, f64)> {
    if points.len() <= max_points {
        return points.to_vec();
    }
    let mut epsilon = SIMPLIFY_EPSILON_DEG;
    let mut simplified = douglas_peucker(points, epsilon);
    while simplified.len() > max_points && epsilon < 0.01 {
        epsilon *= 1.5;
        simplified = douglas_peucker(points, epsilon);
    }
    simplified
}

fn douglas_peucker(points: &[(f64, f64)], epsilon: f64) -> Vec<(f64, f64)> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut max_dist = 0.0_f64;
    let mut index = 0_usize;
    let end = points.len() - 1;
    for i in 1..end {
        let d = perpendicular_distance(points[i], points[0], points[end]);
        if d > max_dist {
            max_dist = d;
            index = i;
        }
    }
    if max_dist > epsilon {
        let mut left = douglas_peucker(&points[..=index], epsilon);
        let right = douglas_peucker(&points[index..], epsilon);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![points[0], points[end]]
    }
}

fn perpendicular_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (plat, plon) = p;
    let (alat, alon) = a;
    let (blat, blon) = b;
    let dx = blon - alon;
    let dy = blat - alat;
    if dx == 0.0 && dy == 0.0 {
        return hypot2(plon - alon, plat - alat);
    }
    let t = ((plon - alon) * dx + (plat - alat) * dy) / (dx * dx + dy * dy);
    let t = t.clamp(0.0, 1.0);
    let proj_lon = alon + t * dx;
    let proj_lat = alat + t * dy;
    hypot2(plon - proj_lon, plat - proj_lat)
}

fn hypot2(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

fn grid_cell_size_degrees(zoom: f64) -> f64 {
    let z = zoom.max(8.0);
    0.08 / z.sqrt()
}

fn round_coord(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn stack_key(lat: f64, lon: f64, decimals: u32) -> (i64, i64) {
    let factor = 10_i64.pow(decimals);
    let rlat = round_coord(lat, decimals);
    let rlon = round_coord(lon, decimals);
    ((rlat * factor as f64) as i64, (rlon * factor as f64) as i64)
}

fn cluster_from_members(members: Vec<ApMarker>) -> ApCluster {
    let count = members.len() as u32;
    let lat = members.iter().map(|m| m.lat).sum::<f64>() / count as f64;
    let lon = members.iter().map(|m| m.lon).sum::<f64>() / count as f64;
    let best = members
        .iter()
        .max_by_key(|m| m.rssi)
        .expect("non-empty members");
    ApCluster {
        lat,
        lon,
        count,
        label_ssid: best.ssid.clone(),
        best_rssi: best.rssi,
        members,
    }
}

fn merge_clusters(groups: &[ApCluster]) -> ApCluster {
    let mut members = Vec::new();
    for group in groups {
        members.extend(group.members.iter().cloned());
    }
    cluster_from_members(members)
}

/// Stack coincident coordinates, then grid-bucket when zoom is below threshold.
#[must_use]
pub fn cluster_map_markers(
    aps: &[ApMarker],
    zoom: f64,
    coord_decimals: u32,
    cluster_zoom_threshold: f64,
) -> Vec<ApCluster> {
    if aps.is_empty() {
        return Vec::new();
    }

    let mut stacks: std::collections::BTreeMap<(i64, i64), Vec<ApMarker>> =
        std::collections::BTreeMap::new();
    for ap in aps {
        stacks
            .entry(stack_key(ap.lat, ap.lon, coord_decimals))
            .or_default()
            .push(ap.clone());
    }

    let stack_clusters: Vec<ApCluster> = stacks.into_values().map(cluster_from_members).collect();

    if zoom >= cluster_zoom_threshold {
        return stack_clusters;
    }

    let cell_size = grid_cell_size_degrees(zoom);
    let mut buckets: std::collections::BTreeMap<(i32, i32), Vec<ApCluster>> =
        std::collections::BTreeMap::new();
    for stack in stack_clusters {
        let key = (
            (stack.lat / cell_size).floor() as i32,
            (stack.lon / cell_size).floor() as i32,
        );
        buckets.entry(key).or_default().push(stack);
    }

    buckets
        .values()
        .map(|groups| merge_clusters(groups))
        .collect()
}

/// Clustered AP marker with member indices into the input slice.
#[derive(Clone, Debug, PartialEq)]
pub struct ApClusterIndexed {
    pub lat: f64,
    pub lon: f64,
    pub count: u32,
    pub label_ssid: String,
    pub best_rssi: i32,
    pub member_indices: Vec<usize>,
}

/// Like [`cluster_map_markers`] but returns member indices instead of full marker copies.
#[must_use]
pub fn cluster_map_markers_indexed(
    aps: &[ApMarker],
    zoom: f64,
    coord_decimals: u32,
    cluster_zoom_threshold: f64,
) -> Vec<ApClusterIndexed> {
    let clusters = cluster_map_markers(aps, zoom, coord_decimals, cluster_zoom_threshold);
    clusters
        .into_iter()
        .map(|c| {
            let member_indices: Vec<usize> = c
                .members
                .iter()
                .filter_map(|m| {
                    aps.iter()
                        .position(|ap| ap.lat == m.lat && ap.lon == m.lon && ap.bssid == m.bssid)
                })
                .collect();
            ApClusterIndexed {
                lat: c.lat,
                lon: c.lon,
                count: c.count,
                label_ssid: c.label_ssid,
                best_rssi: c.best_rssi,
                member_indices,
            }
        })
        .collect()
}
