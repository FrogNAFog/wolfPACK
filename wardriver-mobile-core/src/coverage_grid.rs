//! Phase 2 light: grid coverage shading inside geofence square.

use crate::geofence::GeofenceSquare;

/// Coarse grid cell for coverage map overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridCell {
    pub row: u32,
    pub col: u32,
}

/// Coverage grid over a geofence square.
#[derive(Clone, Debug)]
pub struct CoverageGrid {
    pub square: GeofenceSquare,
    pub rows: u32,
    pub cols: u32,
    pub visited: std::collections::HashSet<GridCell>,
}

impl CoverageGrid {
    #[must_use]
    pub fn new(square: GeofenceSquare, rows: u32, cols: u32) -> Self {
        Self {
            square,
            rows: rows.max(1),
            cols: cols.max(1),
            visited: std::collections::HashSet::new(),
        }
    }

    fn cell_for(&self, lat: f64, lon: f64) -> Option<GridCell> {
        if !self.square.contains(lat, lon) {
            return None;
        }
        let lat_span = self.square.lat_max - self.square.lat_min;
        let lon_span = self.square.lon_max - self.square.lon_min;
        if lat_span <= 0.0 || lon_span <= 0.0 {
            return None;
        }
        let row = ((lat - self.square.lat_min) / lat_span * self.rows as f64) as u32;
        let col = ((lon - self.square.lon_min) / lon_span * self.cols as f64) as u32;
        Some(GridCell {
            row: row.min(self.rows - 1),
            col: col.min(self.cols - 1),
        })
    }

    /// Mark route point as visited.
    pub fn visit(&mut self, lat: f64, lon: f64) {
        if let Some(cell) = self.cell_for(lat, lon) {
            self.visited.insert(cell);
        }
    }

    /// Cells inside the square not yet traversed.
    #[must_use]
    pub fn unvisited_cells(&self) -> Vec<GridCell> {
        let mut out = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let cell = GridCell { row, col };
                if !self.visited.contains(&cell) {
                    out.push(cell);
                }
            }
        }
        out
    }

    /// Fraction of grid cells visited [0.0, 1.0].
    #[must_use]
    pub fn coverage_fraction(&self) -> f64 {
        let total = (self.rows as u64) * (self.cols as u64);
        if total == 0 {
            return 0.0;
        }
        self.visited.len() as f64 / total as f64
    }
}
