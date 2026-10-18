//! Latched home-zone suppression for wardrive (mirrors control `fleet_loop` home geofence latch).
//!
//! `wardrive_suppressed` stays true while latched inside until GPS shows sustained exit.

use crate::mac_table::haversine_m;

/// Sustained GPS-outside duration before clearing the inside latch.
pub const HOME_ZONE_EXIT_SUSTAINED_MS: u64 = 5_000;

/// Stateful home-zone latch (mobile `HomeZoneSuppressor` / control `FleetLoopState` subset).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HomeZoneLatch {
    pub inside_prev: bool,
    pub outside_gps_ok_since_ms: u64,
}

impl HomeZoneLatch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inside_prev: false,
            outside_gps_ok_since_ms: 0,
        }
    }

    /// Stateless point-in-circle test (dashboard / stats).
    #[must_use]
    pub fn contains(center_lat: f64, center_lon: f64, radius_m: f64, lat: f64, lon: f64) -> bool {
        if !lat.is_finite() || !lon.is_finite() {
            return false;
        }
        haversine_m(center_lat, center_lon, lat, lon) <= radius_m
    }

    /// Update latch and return whether wardrive should be suppressed (`inside` home zone).
    ///
    /// When `enabled` is false, resets state and returns false.
    pub fn wardrive_suppressed(
        &mut self,
        enabled: bool,
        center_lat: f64,
        center_lon: f64,
        radius_m: f64,
        lat: Option<f64>,
        lon: Option<f64>,
        gps_ok: bool,
        now_ms: u64,
    ) -> bool {
        if !enabled {
            self.inside_prev = false;
            self.outside_gps_ok_since_ms = 0;
            return false;
        }

        let inside = if self.inside_prev {
            if !gps_ok || lat.is_none() || lon.is_none() {
                self.outside_gps_ok_since_ms = 0;
                true
            } else {
                let lat = lat.unwrap();
                let lon = lon.unwrap();
                let dist_m = haversine_m(center_lat, center_lon, lat, lon);
                if dist_m > radius_m {
                    if self.outside_gps_ok_since_ms == 0 {
                        self.outside_gps_ok_since_ms = now_ms;
                    }
                    let sustained = now_ms.saturating_sub(self.outside_gps_ok_since_ms);
                    if sustained >= HOME_ZONE_EXIT_SUSTAINED_MS {
                        self.inside_prev = false;
                        self.outside_gps_ok_since_ms = 0;
                        false
                    } else {
                        true
                    }
                } else {
                    self.outside_gps_ok_since_ms = 0;
                    true
                }
            }
        } else if gps_ok {
            if let (Some(lat), Some(lon)) = (lat, lon) {
                let dist_m = haversine_m(center_lat, center_lon, lat, lon);
                self.outside_gps_ok_since_ms = 0;
                if dist_m <= radius_m {
                    self.inside_prev = true;
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        inside
    }
}
