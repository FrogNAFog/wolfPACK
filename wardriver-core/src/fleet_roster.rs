//! Pure fleet roster encode/decode and slot assignment (host-testable; no NVS).

/// Max adopted sniffers on the dashboard / NVS roster (single source of truth;
/// `wardriver-stats` re-exports this).
pub const FLEET_MAX_SNIFFERS: usize = 48;

/// NVS blob format version for namespace `fleet_roster` key `entries`.
pub const ROSTER_BLOB_VERSION: u8 = 1;

/// One adopted sniffer: `sniffer_id` + ESP-NOW STA MAC + stable `fleet_slot` (1..=N).
pub const ROSTER_ENTRY_LEN: usize = 4 + 6 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FleetRosterEntry {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub fleet_slot: u8,
}

/// Next free dashboard fleet slot in 1..= [`FLEET_MAX_SNIFFERS`].
#[must_use]
pub fn next_free_fleet_slot(used: &[u8]) -> Option<u8> {
    for slot in 1..=FLEET_MAX_SNIFFERS as u8 {
        if !used.iter().any(|&s| s == slot) {
            return Some(slot);
        }
    }
    None
}

/// Roster match requires both `sniffer_id` and ESP-NOW source MAC (binding invariant).
#[must_use]
pub fn roster_is_adopted(entries: &[FleetRosterEntry], sniffer_id: u32, mac: &[u8; 6]) -> bool {
    entries
        .iter()
        .any(|e| e.sniffer_id == sniffer_id && e.mac == *mac)
}

#[must_use]
pub fn roster_fleet_slot(entries: &[FleetRosterEntry], sniffer_id: u32) -> Option<u8> {
    entries
        .iter()
        .find(|e| e.sniffer_id == sniffer_id)
        .map(|e| e.fleet_slot)
}

/// Assign `fleet_slot` for a new or updated adopt. Returns `None` when the roster is full.
pub fn roster_adopt_entry(
    entries: &mut Vec<FleetRosterEntry>,
    sniffer_id: u32,
    mac: [u8; 6],
) -> Option<u8> {
    if sniffer_id == 0 {
        return None;
    }
    if let Some(idx) = entries.iter().position(|e| e.sniffer_id == sniffer_id) {
        entries[idx].mac = mac;
        return Some(entries[idx].fleet_slot);
    }
    let slot = next_free_fleet_slot(&entries.iter().map(|e| e.fleet_slot).collect::<Vec<_>>())?;
    if entries.len() >= FLEET_MAX_SNIFFERS {
        return None;
    }
    entries.push(FleetRosterEntry {
        sniffer_id,
        mac,
        fleet_slot: slot,
    });
    Some(slot)
}

#[must_use]
pub fn roster_remove_entry(entries: &mut Vec<FleetRosterEntry>, sniffer_id: u32) -> bool {
    if let Some(idx) = entries.iter().position(|e| e.sniffer_id == sniffer_id) {
        entries.remove(idx);
        true
    } else {
        false
    }
}

/// Serialize adopted entries for NVS blob key `entries`.
pub fn encode_roster_blob(entries: &[FleetRosterEntry]) -> Vec<u8> {
    let n = entries.len().min(FLEET_MAX_SNIFFERS);
    let mut out = Vec::with_capacity(2 + n * ROSTER_ENTRY_LEN);
    out.push(ROSTER_BLOB_VERSION);
    out.push(n as u8);
    for e in &entries[..n] {
        out.extend_from_slice(&e.sniffer_id.to_le_bytes());
        out.extend_from_slice(&e.mac);
        out.push(e.fleet_slot);
    }
    out
}

/// Decode NVS blob; invalid or unknown version yields empty roster.
pub fn decode_roster_blob(slice: &[u8]) -> Vec<FleetRosterEntry> {
    if slice.len() < 2 || slice[0] != ROSTER_BLOB_VERSION {
        return Vec::new();
    }
    let n = slice[1] as usize;
    if n > FLEET_MAX_SNIFFERS {
        return Vec::new();
    }
    let need = 2 + n * ROSTER_ENTRY_LEN;
    if slice.len() < need {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n);
    let mut off = 2;
    for _ in 0..n {
        let mut id_b = [0u8; 4];
        id_b.copy_from_slice(&slice[off..off + 4]);
        let sniffer_id = u32::from_le_bytes(id_b);
        off += 4;
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&slice[off..off + 6]);
        off += 6;
        let fleet_slot = slice[off];
        off += 1;
        if sniffer_id != 0 && fleet_slot >= 1 && fleet_slot <= FLEET_MAX_SNIFFERS as u8 {
            out.push(FleetRosterEntry {
                sniffer_id,
                mac,
                fleet_slot,
            });
        }
    }
    out
}
