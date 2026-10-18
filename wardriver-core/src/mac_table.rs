//! Unified MAC table: lane flags, geo-slot budget, and WiGLE row throttle (host-testable).

use indexmap::IndexMap;

/// Per-lane occupancy bits on a single MAC row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LaneFlags(u8);

impl LaneFlags {
    pub const WIGLE_WIFI: u8 = 1 << 0;
    pub const WIGLE_BLE: u8 = 1 << 1;
    pub const DETECTION_WIFI: u8 = 1 << 2;
    pub const DETECTION_BLE: u8 = 1 << 3;
    pub const REMOTE_ID: u8 = 1 << 4;

    #[must_use]
    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }

    pub fn set(&mut self, bit: u8) {
        self.0 |= bit;
    }
}

/// Above this (`rssi >` threshold): repeat rows need only the 50 m move.
pub const WIGLE_RSSI_STRONG_ENOUGH_DBM: i8 = -75;

/// Max WiGLE CSV rows per MAC per boot session (WiFi + BLE lanes share one counter).
pub const WIGLE_MAX_ROWS_PER_MAC: u8 = 10;

#[derive(Clone, Copy, Debug, Default)]
pub struct MacEntry {
    pub lanes: LaneFlags,
    pub geo_active: bool,
    pub last_lat: f64,
    pub last_lon: f64,
    pub wigle_wifi_avg_rssi: Option<i8>,
    pub wigle_wifi_log_count: u8,
    pub wigle_ble_avg_rssi: Option<i8>,
    pub wigle_ble_log_count: u8,
    pub wigle_row_count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WigleRowDecision {
    pub new_unique: bool,
    pub allow_row: bool,
}

/// Side effects from [`MacTable::allow`] for firmware stats wiring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MacTableApplyResult {
    pub decision: WigleRowDecision,
    pub row_evictions: u32,
    pub geo_releases: u32,
}

const EARTH_RADIUS_M: f64 = 6_371_000.0;

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

#[must_use]
pub fn planar_distance_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const M_PER_DEG_LAT: f64 = 111_320.0;
    let dlat_m = (lat2 - lat1) * M_PER_DEG_LAT;
    let phi = ((lat1 + lat2) * 0.5f64).to_radians();
    let m_per_deg_lon = M_PER_DEG_LAT * phi.cos().abs().max(0.2);
    let dlon_m = (lon2 - lon1) * m_per_deg_lon;
    (dlat_m * dlat_m + dlon_m * dlon_m).sqrt()
}

/// True when great-circle distance is at least `min_m` (planar fast-path, haversine tie-break).
#[must_use]
pub fn moved_at_least_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64, min_m: f64) -> bool {
    let p = planar_distance_m(lat1, lon1, lat2, lon2);
    if p < min_m * 0.88 {
        return false;
    }
    if p >= min_m * 1.12 {
        return true;
    }
    haversine_m(lat1, lon1, lat2, lon2) >= min_m
}

#[must_use]
pub fn update_running_avg_i8(prev_avg: i8, prev_count: u8, rssi: i8) -> i8 {
    let n = prev_count as i32 + 1;
    let sum = prev_avg as i32 * prev_count as i32 + rssi as i32;
    (sum / n) as i8
}

#[must_use]
pub fn wigle_rssi_ok_for_repeat(lane_avg: Option<i8>, lane_log_count: u8, rssi: i8) -> bool {
    if rssi > WIGLE_RSSI_STRONG_ENOUGH_DBM {
        return true;
    }
    if lane_log_count == 0 {
        return true;
    }
    lane_avg.is_some_and(|avg| rssi > avg)
}

fn lane_stats_for_wigle(entry: &MacEntry, wigle_lane_bit: u8) -> (Option<i8>, u8) {
    match wigle_lane_bit {
        LaneFlags::WIGLE_WIFI => (entry.wigle_wifi_avg_rssi, entry.wigle_wifi_log_count),
        LaneFlags::WIGLE_BLE => (entry.wigle_ble_avg_rssi, entry.wigle_ble_log_count),
        _ => (None, 0),
    }
}

/// Shared lane-decision core: finite guard + `new_unique` + the no-entry arm.
/// The per-lane repeat policy (the behavior-specific arms) is supplied by `repeat`.
fn lane_decision_with_repeat<F>(
    entry: Option<&MacEntry>,
    lat: f64,
    lon: f64,
    lane_bit: u8,
    repeat: F,
) -> WigleRowDecision
where
    F: FnOnce(&MacEntry) -> bool,
{
    if !lat.is_finite() || !lon.is_finite() {
        return WigleRowDecision {
            new_unique: false,
            allow_row: false,
        };
    }
    let new_unique = match entry {
        None => true,
        Some(e) => !e.lanes.has(lane_bit),
    };
    let allow_row = match entry {
        None => true,
        Some(e) => repeat(e),
    };
    WigleRowDecision {
        new_unique,
        allow_row,
    }
}

fn consider_wigle_lane(
    entry: Option<&MacEntry>,
    lat: f64,
    lon: f64,
    revisit_radius_m: f64,
    wigle_lane_bit: u8,
    rssi: i8,
) -> WigleRowDecision {
    lane_decision_with_repeat(entry, lat, lon, wigle_lane_bit, |e| {
        if e.wigle_row_count >= WIGLE_MAX_ROWS_PER_MAC {
            false
        } else if !e.lanes.has(wigle_lane_bit) {
            true
        } else {
            let moved = moved_at_least_m(e.last_lat, e.last_lon, lat, lon, revisit_radius_m);
            let (lane_avg, lane_log_count) = lane_stats_for_wigle(e, wigle_lane_bit);
            moved && wigle_rssi_ok_for_repeat(lane_avg, lane_log_count, rssi)
        }
    })
}

fn consider_detection_lane(
    entry: Option<&MacEntry>,
    lat: f64,
    lon: f64,
    revisit_radius_m: f64,
    detection_lane_bit: u8,
) -> WigleRowDecision {
    lane_decision_with_repeat(entry, lat, lon, detection_lane_bit, |e| {
        if !e.geo_active {
            true
        } else {
            moved_at_least_m(e.last_lat, e.last_lon, lat, lon, revisit_radius_m)
        }
    })
}

#[must_use]
pub fn consider_wigle_wifi(
    entry: Option<&MacEntry>,
    lat: f64,
    lon: f64,
    revisit_radius_m: f64,
    rssi: i8,
) -> WigleRowDecision {
    consider_wigle_lane(
        entry,
        lat,
        lon,
        revisit_radius_m,
        LaneFlags::WIGLE_WIFI,
        rssi,
    )
}

#[must_use]
pub fn consider_wigle_ble(
    entry: Option<&MacEntry>,
    lat: f64,
    lon: f64,
    revisit_radius_m: f64,
    rssi: i8,
) -> WigleRowDecision {
    consider_wigle_lane(
        entry,
        lat,
        lon,
        revisit_radius_m,
        LaneFlags::WIGLE_BLE,
        rssi,
    )
}

#[derive(Debug)]
pub struct MacTable {
    entries: IndexMap<[u8; 6], MacEntry>,
    max_entries: usize,
    geo_slots_max: usize,
    geo_active_count: usize,
}

impl MacTable {
    #[must_use]
    pub fn new(max_entries: usize, geo_slots_max: usize) -> Self {
        Self {
            entries: IndexMap::new(),
            max_entries,
            geo_slots_max,
            geo_active_count: 0,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn geo_active_count(&self) -> usize {
        self.geo_active_count
    }

    #[must_use]
    pub fn get(&self, mac: &[u8; 6]) -> Option<&MacEntry> {
        self.entries.get(mac)
    }

    /// Hydrate revisit anchors from persisted `mac_last` (does not increment [`MacEntry::wigle_row_count`]).
    pub fn seed_wigle_lanes(&mut self, mac: &[u8; 6], lat: f64, lon: f64) {
        if !lat.is_finite() || !lon.is_finite() {
            return;
        }
        let prev = self.entries.get(mac).copied();
        if prev.is_none() {
            let _ = self.ensure_capacity_for_new_row();
        } else {
            self.entries.shift_remove(mac);
        }
        let mut entry = prev.unwrap_or_default();
        let needs_geo_slot = prev.map(|e| !e.geo_active).unwrap_or(true);
        if needs_geo_slot {
            let _ = self.acquire_geo_slot();
            self.geo_active_count = self.geo_active_count.saturating_add(1);
        }
        entry.lanes.set(LaneFlags::WIGLE_WIFI);
        entry.lanes.set(LaneFlags::WIGLE_BLE);
        entry.geo_active = true;
        entry.last_lat = lat;
        entry.last_lon = lon;
        self.entries.insert(*mac, entry);
    }

    pub fn remove(&mut self, mac: &[u8; 6]) -> bool {
        if let Some(entry) = self.entries.shift_remove(mac) {
            if entry.geo_active {
                self.geo_active_count = self.geo_active_count.saturating_sub(1);
            }
            return true;
        }
        false
    }

    /// Drop LRU full rows until at most `keep_fraction` × `max_entries` remain.
    pub fn shrink_to_cap_fraction(&mut self, keep_fraction: f64) -> u32 {
        let keep_fraction = keep_fraction.clamp(0.1, 1.0);
        let target =
            ((self.max_entries as f64 * keep_fraction).ceil() as usize).min(self.max_entries);
        let mut evicted = 0u32;
        while self.entries.len() > target {
            if self.evict_full_row_lru() {
                evicted = evicted.saturating_add(1);
            } else {
                break;
            }
        }
        evicted
    }

    /// WiGLE / LPR-camera row throttle. Pass `Some(rssi)` for [`LaneFlags::WIGLE_WIFI`] / [`LaneFlags::WIGLE_BLE`].
    pub fn allow(
        &mut self,
        mac: &[u8; 6],
        lat: f64,
        lon: f64,
        lane: u8,
        revisit_radius_m: f64,
        rssi: Option<i8>,
    ) -> MacTableApplyResult {
        let prev = self.entries.get(mac).copied();
        let decision = match lane {
            LaneFlags::WIGLE_WIFI => {
                let Some(rssi) = rssi else {
                    return MacTableApplyResult {
                        decision: WigleRowDecision {
                            new_unique: false,
                            allow_row: false,
                        },
                        row_evictions: 0,
                        geo_releases: 0,
                    };
                };
                consider_wigle_wifi(prev.as_ref(), lat, lon, revisit_radius_m, rssi)
            }
            LaneFlags::WIGLE_BLE => {
                let Some(rssi) = rssi else {
                    return MacTableApplyResult {
                        decision: WigleRowDecision {
                            new_unique: false,
                            allow_row: false,
                        },
                        row_evictions: 0,
                        geo_releases: 0,
                    };
                };
                consider_wigle_ble(prev.as_ref(), lat, lon, revisit_radius_m, rssi)
            }
            LaneFlags::DETECTION_WIFI => consider_detection_lane(
                prev.as_ref(),
                lat,
                lon,
                revisit_radius_m,
                LaneFlags::DETECTION_WIFI,
            ),
            LaneFlags::DETECTION_BLE => consider_detection_lane(
                prev.as_ref(),
                lat,
                lon,
                revisit_radius_m,
                LaneFlags::DETECTION_BLE,
            ),
            LaneFlags::REMOTE_ID => consider_detection_lane(
                prev.as_ref(),
                lat,
                lon,
                revisit_radius_m,
                LaneFlags::REMOTE_ID,
            ),
            _ => WigleRowDecision {
                new_unique: false,
                allow_row: false,
            },
        };

        if !decision.allow_row {
            return MacTableApplyResult {
                decision,
                row_evictions: 0,
                geo_releases: 0,
            };
        }

        let mut row_evictions = 0u32;
        let mut geo_releases = 0u32;

        let row_absent = prev.is_none();
        if row_absent {
            row_evictions = row_evictions.saturating_add(self.ensure_capacity_for_new_row());
        } else {
            self.entries.shift_remove(mac);
        }

        let needs_geo_slot = prev.map(|e| !e.geo_active).unwrap_or(true);
        if needs_geo_slot {
            geo_releases = geo_releases.saturating_add(self.acquire_geo_slot());
        }

        let entry = prev.unwrap_or_default();
        let mut updated = entry;
        updated.lanes.set(lane);
        updated.geo_active = true;
        updated.last_lat = lat;
        updated.last_lon = lon;
        if lane == LaneFlags::WIGLE_WIFI {
            if let Some(rssi) = rssi {
                let prev_count = updated.wigle_wifi_log_count;
                updated.wigle_wifi_avg_rssi = Some(match updated.wigle_wifi_avg_rssi {
                    Some(avg) => update_running_avg_i8(avg, prev_count, rssi),
                    None => rssi,
                });
                updated.wigle_wifi_log_count = prev_count.saturating_add(1);
            }
            updated.wigle_row_count = updated.wigle_row_count.saturating_add(1);
        } else if lane == LaneFlags::WIGLE_BLE {
            if let Some(rssi) = rssi {
                let prev_count = updated.wigle_ble_log_count;
                updated.wigle_ble_avg_rssi = Some(match updated.wigle_ble_avg_rssi {
                    Some(avg) => update_running_avg_i8(avg, prev_count, rssi),
                    None => rssi,
                });
                updated.wigle_ble_log_count = prev_count.saturating_add(1);
            }
            updated.wigle_row_count = updated.wigle_row_count.saturating_add(1);
        }
        if needs_geo_slot {
            self.geo_active_count = self.geo_active_count.saturating_add(1);
        }
        self.entries.insert(*mac, updated);

        MacTableApplyResult {
            decision,
            row_evictions,
            geo_releases,
        }
    }

    fn ensure_capacity_for_new_row(&mut self) -> u32 {
        let mut evicted = 0u32;
        while self.entries.len() >= self.max_entries {
            if self.evict_full_row_lru() {
                evicted = evicted.saturating_add(1);
            } else {
                break;
            }
        }
        evicted
    }

    fn acquire_geo_slot(&mut self) -> u32 {
        let mut released = 0u32;
        while self.geo_active_count >= self.geo_slots_max {
            if self.release_geo_slot_lru() {
                released = released.saturating_add(1);
            } else {
                break;
            }
        }
        released
    }

    fn evict_full_row_lru(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        if let Some((_, entry)) = self.entries.shift_remove_index(0) {
            if entry.geo_active {
                self.geo_active_count = self.geo_active_count.saturating_sub(1);
            }
            return true;
        }
        false
    }

    fn release_geo_slot_lru(&mut self) -> bool {
        for i in 0..self.entries.len() {
            let geo_active = self.entries.get_index(i).is_some_and(|(_, e)| e.geo_active);
            if geo_active {
                if let Some((_, entry)) = self.entries.get_index_mut(i) {
                    entry.geo_active = false;
                    self.geo_active_count = self.geo_active_count.saturating_sub(1);
                    return true;
                }
            }
        }
        false
    }
}
