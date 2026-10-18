//! Phase 5 optional: OSM street coverage routing inside geofence square (stub API).

use crate::geofence::GeofenceSquare;

/// Waypoint on a planned coverage route.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RouteWaypoint {
    pub lat: f64,
    pub lon: f64,
}

/// Street-graph coverage solver placeholder.
///
/// Full OSM Chinese Postman routing is deferred; callers use [`CoverageGrid`](crate::coverage_grid::CoverageGrid) for Phase 2 light feedback.
#[derive(Clone, Debug, Default)]
pub struct StreetCoverageRouter {
    pub enabled: bool,
}

impl StreetCoverageRouter {
    #[must_use]
    pub const fn new() -> Self {
        Self { enabled: false }
    }

    /// Returns an empty route until OSM graph integration lands.
    #[must_use]
    pub fn plan_coverage_route(
        &self,
        _square: &GeofenceSquare,
        _start_lat: f64,
        _start_lon: f64,
    ) -> Vec<RouteWaypoint> {
        if !self.enabled {
            return Vec::new();
        }
        Vec::new()
    }
}
