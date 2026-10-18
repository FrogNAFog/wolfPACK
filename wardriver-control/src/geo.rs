//! Great-circle distance for GPS geofencing (no external crates).

const EARTH_RADIUS_M: f64 = 6_371_000.0;

/// Haversine distance in meters between two WGS84 points (degrees).
#[must_use]
pub fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let deg = std::f64::consts::PI / 180.0;
    let phi1 = lat1 * deg;
    let phi2 = lat2 * deg;
    let dphi = (lat2 - lat1) * deg;
    let dlambda = (lon2 - lon1) * deg;
    let a = (dphi / 2.0).sin().mul_add(
        (dphi / 2.0).sin(),
        phi1.cos() * phi2.cos() * (dlambda / 2.0).sin() * (dlambda / 2.0).sin(),
    );
    let c = 2.0 * a.sqrt().atan2((1.0 - a).max(0.0).sqrt());
    EARTH_RADIUS_M * c
}

/// Planar (Euclidean) approximation in meters for short baselines; cheaper than [`haversine_m`].
///
/// Uses mean latitude for longitude scaling. For the small distances used in WiGLE row
/// throttling (~10–100 m), this tracks haversine closely enough for fast reject/accept.
#[must_use]
pub fn planar_distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const M_PER_DEG_LAT: f64 = 111_320.0;
    let dlat_m = (lat2 - lat1) * M_PER_DEG_LAT;
    let phi = ((lat1 + lat2) * 0.5f64).to_radians();
    let m_per_deg_lon = M_PER_DEG_LAT * phi.cos().abs().max(0.2);
    let dlon_m = (lon2 - lon1) * m_per_deg_lon;
    (dlat_m * dlat_m + dlon_m * dlon_m).sqrt()
}

/// True iff great-circle distance is at least `min_m`, using [`planar_distance_m`] to skip
/// haversine when the answer is obvious.
#[must_use]
pub fn moved_at_least_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64, min_m: f64) -> bool {
    let p = planar_distance_m(lat1, lon1, lat2, lon2);
    // Conservative: never claim "moved enough" from planar alone unless planar clearly exceeds.
    if p < min_m * 0.88 {
        return false;
    }
    if p >= min_m * 1.12 {
        return true;
    }
    haversine_m(lat1, lon1, lat2, lon2) >= min_m
}
