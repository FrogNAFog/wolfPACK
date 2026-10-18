//! Cell-tower revisit dedup keyed by WiGLE composite cell key (string).

use indexmap::IndexMap;
use wardriver_core::mac_table::{moved_at_least_m, WigleRowDecision};

#[derive(Clone, Copy, Debug, Default)]
pub struct CellEntry {
    pub last_lat: f64,
    pub last_lon: f64,
}

/// 50 m revisit throttle for phone-sourced cell towers (mirrors MacTable WiGLE lanes).
#[derive(Debug)]
pub struct CellTable {
    rows: IndexMap<String, CellEntry>,
    cap: usize,
}

impl CellTable {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            rows: IndexMap::new(),
            cap: cap.max(1),
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CellEntry> {
        self.rows.get(key)
    }

    /// Decide and update last position for `key` at (`lat`, `lon`).
    #[must_use]
    pub fn allow(
        &mut self,
        key: &str,
        lat: f64,
        lon: f64,
        revisit_radius_m: f64,
    ) -> WigleRowDecision {
        if !lat.is_finite() || !lon.is_finite() {
            return WigleRowDecision {
                new_unique: false,
                allow_row: false,
            };
        }
        let new_unique = !self.rows.contains_key(key);
        let allow_row = match self.rows.get(key) {
            None => true,
            Some(e) => moved_at_least_m(e.last_lat, e.last_lon, lat, lon, revisit_radius_m),
        };
        if allow_row {
            if new_unique && self.rows.len() >= self.cap {
                self.rows.shift_remove_index(0);
            }
            self.rows.insert(
                key.to_string(),
                CellEntry {
                    last_lat: lat,
                    last_lon: lon,
                },
            );
        }
        WigleRowDecision {
            new_unique,
            allow_row,
        }
    }

    /// Hydrate from prior summaries: `cell_key` + last lat/lon (no row emitted).
    pub fn seed(&mut self, key: &str, lat: f64, lon: f64) {
        if !lat.is_finite() || !lon.is_finite() {
            return;
        }
        if self.rows.len() >= self.cap && !self.rows.contains_key(key) {
            self.rows.shift_remove_index(0);
        }
        self.rows.insert(
            key.to_string(),
            CellEntry {
                last_lat: lat,
                last_lon: lon,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisit_blocks_same_cell_within_radius() {
        let mut t = CellTable::new(64);
        let d1 = t.allow("310410_1_2", 37.0, -122.0, 50.0);
        assert!(d1.allow_row);
        let d2 = t.allow("310410_1_2", 37.0001, -122.0001, 50.0);
        assert!(!d2.allow_row);
    }

    #[test]
    fn revisit_allows_after_move() {
        let mut t = CellTable::new(64);
        assert!(t.allow("310410_1_2", 37.0, -122.0, 50.0).allow_row);
        assert!(t.allow("310410_1_2", 37.001, -122.001, 50.0).allow_row);
    }
}
