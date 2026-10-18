//! Mobile GPS fix timeline — control-parity retention and anchor matching buffer.
//!
//! Mirrors Android [`GpsTimeline`]: 500 ms sampling, five-minute retention, fix-time keys.

use std::collections::VecDeque;

use crate::gps_anchor::GpsFix;
use crate::gps_anchor_mobile::MAX_MOBILE_ACCURACY_M;

/// Matches control [`GPS_HIST_RETENTION_MS`] (five minutes).
pub const GPS_HIST_RETENTION_MS: i64 = 300_000;

/// Reject fixes more than one minute ahead of local clock.
pub const MAX_FUTURE_SKEW_MS: i64 = 60_000;

/// One GPS sample for PackLink observation anchor matching.
#[derive(Clone, Debug, PartialEq)]
pub struct GpsSample {
    pub unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<f64>,
    pub accuracy_m: f32,
}

/// Control-parity GPS timeline keyed by fix timestamp.
#[derive(Clone, Debug, Default)]
pub struct GpsTimeline {
    samples: Vec<GpsSample>,
}

impl GpsTimeline {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: Vec::new(),
        }
    }

    /// Reject fixes more than one minute ahead of local clock.
    #[must_use]
    pub fn is_valid_fix_time(unix_ms: i64, now_ms: i64) -> bool {
        unix_ms > 0 && unix_ms <= now_ms + MAX_FUTURE_SKEW_MS
    }

    /// Replay bulk older than control retention — distinct from live `no_gps_match`.
    #[must_use]
    pub fn is_stale_replay(hdr_scan_end_unix_ms: i64, now_ms: i64) -> bool {
        hdr_scan_end_unix_ms > 0
            && now_ms.saturating_sub(hdr_scan_end_unix_ms) > GPS_HIST_RETENTION_MS
    }

    #[must_use]
    pub fn is_usable(accuracy_m: f32) -> bool {
        accuracy_m > 0.0 && accuracy_m <= MAX_MOBILE_ACCURACY_M
    }

    #[must_use]
    pub fn latest_usable(&self) -> bool {
        !self.samples.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn append(
        &mut self,
        unix_ms: i64,
        lat: f64,
        lon: f64,
        accuracy_m: f32,
        altitude_m: Option<f64>,
        now_ms: i64,
    ) -> bool {
        if !Self::is_valid_fix_time(unix_ms, now_ms) {
            return false;
        }
        if !Self::is_usable(accuracy_m) {
            return false;
        }
        let sample = GpsSample {
            unix_ms,
            lat,
            lon,
            altitude_m,
            accuracy_m,
        };
        if let Some(last) = self.samples.last() {
            if last.unix_ms == unix_ms {
                let idx = self.samples.len() - 1;
                self.samples[idx] = sample;
            } else {
                self.samples.push(sample);
                self.samples.sort_by_key(|s| s.unix_ms);
            }
        } else {
            self.samples.push(sample);
        }
        self.prune(now_ms);
        true
    }

    /// Merge route points (e.g. from SQLite replay hydration).
    pub fn merge_samples<I>(&mut self, points: I, now_ms: i64)
    where
        I: IntoIterator<Item = (i64, f64, f64, f32, Option<f64>)>,
    {
        for (unix_ms, lat, lon, accuracy_m, altitude_m) in points {
            self.append(unix_ms, lat, lon, accuracy_m, altitude_m, now_ms);
        }
    }

    pub fn prune(&mut self, now_ms: i64) {
        let cutoff = now_ms - GPS_HIST_RETENTION_MS;
        self.samples.retain(|s| s.unix_ms >= cutoff);
    }

    /// Convert to `gps_hist` deque for enrichment anchor matching.
    #[must_use]
    pub fn to_fix_hist(&self) -> VecDeque<(i64, GpsFix)> {
        self.samples
            .iter()
            .map(|s| {
                (
                    s.unix_ms,
                    GpsFix {
                        utc_unix_ms: s.unix_ms,
                        lat: s.lat,
                        lon: s.lon,
                        altitude_m: s.altitude_m.map(|a| a as f32),
                        hdop: None,
                        accuracy_m: Some(s.accuracy_m),
                        fix_quality: 1,
                    },
                )
            })
            .collect()
    }
}
