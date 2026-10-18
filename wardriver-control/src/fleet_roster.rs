//! NVS-backed adopted sniffer roster.

use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::warn;
use wardriver_core::fleet_roster::{
    decode_roster_blob, encode_roster_blob, roster_adopt_entry, roster_fleet_slot,
    roster_is_adopted, roster_remove_entry, FleetRosterEntry, FLEET_MAX_SNIFFERS, ROSTER_ENTRY_LEN,
};

const NS: &str = "fleet_roster";
const K_ENTRIES: &str = "entries";

/// In-memory roster mirrored to NVS namespace [`NS`].
#[derive(Clone, Debug, Default)]
pub struct FleetRoster {
    entries: Vec<FleetRosterEntry>,
}

impl FleetRoster {
    pub fn load(partition: &EspNvsPartition<NvsDefault>) -> Self {
        let nvs = match EspNvs::new(partition.clone(), NS, true) {
            Ok(n) => n,
            Err(e) => {
                warn!("fleet_roster: open NVS: {e:?}");
                return Self::default();
            }
        };
        let mut raw = [0u8; 2 + ROSTER_ENTRY_LEN * FLEET_MAX_SNIFFERS];
        let slice = match nvs.get_blob(K_ENTRIES, &mut raw) {
            Ok(Some(s)) => s,
            Ok(None) => return Self::default(),
            Err(e) => {
                warn!("fleet_roster: read entries: {e:?}");
                return Self::default();
            }
        };
        Self {
            entries: decode_roster_blob(slice),
        }
    }

    pub fn save(&self, partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
        let blob = encode_roster_blob(&self.entries);
        let nvs =
            EspNvs::new(partition.clone(), NS, true).context("open fleet_roster NVS for write")?;
        nvs.set_blob(K_ENTRIES, blob.as_slice())
            .map_err(|e| anyhow!("nvs set_blob fleet_roster entries: {e:?}"))?;
        Ok(())
    }

    #[must_use]
    pub fn is_adopted(&self, sniffer_id: u32, mac: &[u8; 6]) -> bool {
        roster_is_adopted(&self.entries, sniffer_id, mac)
    }

    pub fn adopt(&mut self, sniffer_id: u32, mac: [u8; 6]) -> Result<u8> {
        roster_adopt_entry(&mut self.entries, sniffer_id, mac)
            .ok_or_else(|| anyhow!("fleet roster full or invalid sniffer_id"))
    }

    #[must_use]
    pub fn remove(&mut self, sniffer_id: u32) -> bool {
        roster_remove_entry(&mut self.entries, sniffer_id)
    }

    #[must_use]
    pub fn list(&self) -> &[FleetRosterEntry] {
        &self.entries
    }

    #[must_use]
    pub fn fleet_slot_for(&self, sniffer_id: u32) -> Option<u8> {
        roster_fleet_slot(&self.entries, sniffer_id)
    }

    /// Remove all adopted entries; returns `(sniffer_id, mac)` pairs that were cleared.
    pub fn clear_all(&mut self) -> Vec<(u32, [u8; 6])> {
        let removed: Vec<(u32, [u8; 6])> =
            self.entries.iter().map(|e| (e.sniffer_id, e.mac)).collect();
        self.entries.clear();
        removed
    }
}

static ROSTER: Mutex<Option<FleetRoster>> = Mutex::new(None);

pub fn init(partition: &EspNvsPartition<NvsDefault>) {
    if let Ok(mut g) = ROSTER.lock() {
        *g = Some(FleetRoster::load(partition));
    }
}

pub fn with_roster<R>(f: impl FnOnce(&FleetRoster) -> R) -> Option<R> {
    ROSTER.lock().ok().and_then(|g| g.as_ref().map(f))
}

pub fn with_roster_mut<R>(f: impl FnOnce(&mut FleetRoster) -> R) -> Option<R> {
    ROSTER.lock().ok().and_then(|mut g| g.as_mut().map(f))
}

pub fn save_roster(partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
    let blob = roster_blob_snapshot()?;
    write_roster_blob(partition, &blob)
}

/// Encode the current in-memory roster for deferred NVS write (snapshot under mutex).
pub fn roster_blob_snapshot() -> Result<Vec<u8>> {
    let roster = ROSTER
        .lock()
        .map_err(|_| anyhow!("fleet roster lock poison"))?;
    let Some(r) = roster.as_ref() else {
        return Err(anyhow!("fleet roster not initialized"));
    };
    Ok(encode_roster_blob(&r.entries))
}

pub fn write_roster_blob(partition: &EspNvsPartition<NvsDefault>, blob: &[u8]) -> Result<()> {
    let nvs =
        EspNvs::new(partition.clone(), NS, true).context("open fleet_roster NVS for write")?;
    nvs.set_blob(K_ENTRIES, blob)
        .map_err(|e| anyhow!("nvs set_blob fleet_roster entries: {e:?}"))?;
    Ok(())
}
