//! Geofence square monitoring (MVP leave-square alert).

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeofenceSquare {
    pub lat_min: f64,
    pub lat_max: f64,
    pub lon_min: f64,
    pub lon_max: f64,
}

impl GeofenceSquare {
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            lat_min: 0.0,
            lat_max: 0.0,
            lon_min: 0.0,
            lon_max: 0.0,
        }
    }

    #[must_use]
    pub fn contains(&self, lat: f64, lon: f64) -> bool {
        lat >= self.lat_min && lat <= self.lat_max && lon >= self.lon_min && lon <= self.lon_max
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeofenceEvent {
    Entered,
    Left,
    None,
}

/// Track inside/outside transitions for leave-square notifications.
#[derive(Clone, Debug)]
pub struct GeofenceMonitor {
    pub square: GeofenceSquare,
    pub enabled: bool,
    pub was_inside: Option<bool>,
}

impl GeofenceMonitor {
    #[must_use]
    pub fn new(square: GeofenceSquare, enabled: bool) -> Self {
        Self {
            square,
            enabled,
            was_inside: None,
        }
    }

    /// Update with latest GPS fix; returns `Left` on exit during active session.
    pub fn update(&mut self, lat: f64, lon: f64) -> GeofenceEvent {
        if !self.enabled {
            return GeofenceEvent::None;
        }
        let inside = self.square.contains(lat, lon);
        let ev = match self.was_inside {
            None => {
                if inside {
                    GeofenceEvent::Entered
                } else {
                    GeofenceEvent::None
                }
            }
            Some(true) if !inside => GeofenceEvent::Left,
            Some(false) if inside => GeofenceEvent::Entered,
            _ => GeofenceEvent::None,
        };
        self.was_inside = Some(inside);
        ev
    }
}
