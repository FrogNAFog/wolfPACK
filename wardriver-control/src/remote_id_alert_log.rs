//! Track MACs that have fired a Remote ID alert and emit follow-up PACKcsv rows using the
//! unified [`crate::csv::MacTable`] geo throttle (`REMOTE_ID` lane).

use indexmap::IndexMap;

use crate::csv;

/// Metadata captured on first alert; reused for follow-up `detection_method_id` / `signal_kind`.
#[derive(Clone, Copy, Debug)]
pub struct RemoteIdAlertMeta {
    pub method_id: u8,
    pub signal_kind: u8,
}

/// Per-stack Remote ID MAC registry; geo throttle lives in the shared [`MacTable`].
pub struct RemoteIdAlertRegistry {
    meta: IndexMap<[u8; 6], RemoteIdAlertMeta>,
    cap: usize,
}

pub type RemoteIdAlertLog = RemoteIdAlertRegistry;

impl RemoteIdAlertRegistry {
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

    pub fn remember(&mut self, mac: [u8; 6], method_id: u8, signal_kind: u8) {
        let entry = RemoteIdAlertMeta {
            method_id,
            signal_kind,
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

    pub fn try_followup(
        &mut self,
        mac: &[u8; 6],
        lat: f64,
        lon: f64,
        table: &mut csv::MacTable,
    ) -> Option<RemoteIdAlertMeta> {
        if !self.meta.contains_key(mac) {
            return None;
        }
        if !table.allow_remote_id(mac, lat, lon) {
            return None;
        }
        let prev = self.meta.shift_remove(mac)?;
        self.meta.insert(*mac, prev);
        Some(prev)
    }
}
