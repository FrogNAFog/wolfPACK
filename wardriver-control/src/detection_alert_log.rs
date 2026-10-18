//! Track MACs that have fired a Flock alert and emit follow-up Detection CSV rows using the
//! unified [`crate::csv::MacTable`] geo throttle (DETECTION_WIFI / DETECTION_BLE lanes).

use indexmap::IndexMap;

use crate::csv;
use wardriver_core::detection::DetectionSignalKind;
use wardriver_core::mac_table::LaneFlags;

/// Metadata captured on first alert; reused for follow-up `detection_method_id` / `signal_kind`.
#[derive(Clone, Copy, Debug)]
pub struct DetectionAlertMeta {
    pub method_id: u8,
    pub signal_kind: u8,
    /// Route follow-ups to `Axon-*.mdetectioncsv` when true (PACK mobile surveillance).
    pub is_mobile_surveillance: bool,
}

/// Per-stack (WiFi vs BLE) alert MAC registry; geo throttle lives in the shared [`MacTable`].
pub struct DetectionAlertRegistry {
    meta: IndexMap<[u8; 6], DetectionAlertMeta>,
    cap: usize,
}

/// Backward-compatible name for ingest call sites.
pub type DetectionAlertLog = DetectionAlertRegistry;

impl DetectionAlertRegistry {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            meta: IndexMap::new(),
            cap: cap.max(16),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.meta.len()
    }

    #[must_use]
    pub fn contains_mac(&self, mac: &[u8; 6]) -> bool {
        self.meta.contains_key(mac)
    }

    /// Peek alert metadata without consuming geo-throttle budget.
    #[must_use]
    pub fn meta(&self, mac: &[u8; 6]) -> Option<DetectionAlertMeta> {
        self.meta.get(mac).copied()
    }

    /// Record that `mac` has alerted; evicts LRU meta when over capacity (no MacTable touch).
    pub fn remember(
        &mut self,
        mac: [u8; 6],
        method_id: u8,
        signal_kind: u8,
        is_mobile_surveillance: bool,
    ) {
        let entry = DetectionAlertMeta {
            method_id,
            signal_kind,
            is_mobile_surveillance,
        };
        if self.meta.contains_key(&mac) {
            self.meta.shift_remove(&mac);
        } else {
            while self.meta.len() >= self.cap {
                if self.meta.shift_remove_index(0).is_none() {
                    break;
                }
            }
        }
        self.meta.insert(mac, entry);
    }

    /// If `mac` is tracked and the shared MacTable allows a follow-up at `(lat, lon)`, return meta.
    pub fn try_followup(
        &mut self,
        mac: &[u8; 6],
        lat: f64,
        lon: f64,
        table: &mut csv::MacTable,
    ) -> Option<DetectionAlertMeta> {
        if !self.meta.contains_key(mac) {
            return None;
        }
        let detection_lane = self
            .meta
            .get(mac)
            .map(|m| {
                if m.signal_kind == DetectionSignalKind::Ble.as_u8() {
                    LaneFlags::DETECTION_BLE
                } else {
                    LaneFlags::DETECTION_WIFI
                }
            })
            .unwrap_or(LaneFlags::DETECTION_WIFI);
        if !table.allow_detection(mac, lat, lon, detection_lane) {
            return None;
        }
        let prev = self.meta.shift_remove(mac)?;
        self.meta.insert(*mac, prev);
        Some(prev)
    }
}
