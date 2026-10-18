//! Optional per-entry TLV tail on scan/BLE fragment wire rows.

/// Roaming Consortium IE body (`ie.getBytes()` in WiGLE Android).
pub const SCAN_TLV_TAG_RCOI_IE: u8 = 0x01;
/// BLE Class of Device AD payload (3 bytes, little-endian 24-bit).
pub const SCAN_TLV_TAG_BLE_COD: u8 = 0x03;

/// Optional enrichment bytes carried after one WiFi AP or BLE entry on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanEntryExtras {
    pub rcoi_ie_payload: Vec<u8>,
    pub ble_cod: Option<[u8; 3]>,
}

impl ScanEntryExtras {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rcoi_ie_payload.is_empty() && self.ble_cod.is_none()
    }
}

/// On-wire byte length of TLV tail for `extras` (0 when empty).
#[must_use]
pub fn scan_entry_tlv_wire_len(extras: &ScanEntryExtras) -> usize {
    let mut n = 0usize;
    if !extras.rcoi_ie_payload.is_empty() {
        let len = extras.rcoi_ie_payload.len();
        if len <= 255 {
            n = n.saturating_add(2 + len);
        }
    }
    if let Some(_cod) = extras.ble_cod {
        n = n.saturating_add(2 + 3);
    }
    n
}

/// Append TLV tail bytes for `extras` to `out`.
pub fn pack_scan_entry_tlv(extras: &ScanEntryExtras, out: &mut Vec<u8>) {
    if !extras.rcoi_ie_payload.is_empty() {
        let len = extras.rcoi_ie_payload.len();
        if len <= 255 {
            out.push(SCAN_TLV_TAG_RCOI_IE);
            out.push(len as u8);
            out.extend_from_slice(&extras.rcoi_ie_payload);
        }
    }
    if let Some(cod) = extras.ble_cod {
        out.push(SCAN_TLV_TAG_BLE_COD);
        out.push(3);
        out.extend_from_slice(&cod);
    }
}

fn is_known_tlv_tag(tag: u8) -> bool {
    matches!(tag, SCAN_TLV_TAG_RCOI_IE | SCAN_TLV_TAG_BLE_COD)
}

/// Parse TLV bytes at `b[*i..]` until an unknown tag or end of buffer.
pub fn parse_scan_entry_tlv(b: &[u8], i: &mut usize) -> ScanEntryExtras {
    let mut extras = ScanEntryExtras::default();
    while *i < b.len() {
        let tag = b[*i];
        if !is_known_tlv_tag(tag) {
            break;
        }
        if *i + 2 > b.len() {
            break;
        }
        let len = b[*i + 1] as usize;
        if *i + 2 + len > b.len() {
            break;
        }
        let val = &b[*i + 2..*i + 2 + len];
        match tag {
            SCAN_TLV_TAG_RCOI_IE => extras.rcoi_ie_payload = val.to_vec(),
            SCAN_TLV_TAG_BLE_COD if len == 3 => {
                extras.ble_cod = Some([val[0], val[1], val[2]]);
            }
            _ => {}
        }
        *i += 2 + len;
    }
    extras
}

fn looks_like_wifi_ap_entry_start(b: &[u8], i: usize) -> bool {
    if i + crate::AP_ENTRY_WIRE_OVERHEAD > b.len() {
        return false;
    }
    let ssid_len = b[i + 9] as usize;
    if ssid_len > 32 {
        return false;
    }
    i + crate::AP_ENTRY_WIRE_OVERHEAD + ssid_len <= b.len()
}

fn looks_like_ble_entry_start(b: &[u8], i: usize) -> bool {
    i + crate::BLE_ENTRY_SIZE <= b.len()
}

/// Parse optional TLV tail after one WiFi AP row; rewinds when bytes belong to the next AP.
pub fn parse_wifi_ap_entry_tlv(b: &[u8], i: &mut usize, more_aps_follow: bool) -> ScanEntryExtras {
    if *i >= b.len() {
        return ScanEntryExtras::default();
    }
    let start = *i;
    let extras = parse_scan_entry_tlv(b, i);
    if more_aps_follow && !looks_like_wifi_ap_entry_start(b, *i) {
        *i = start;
        return ScanEntryExtras::default();
    }
    extras
}

/// Parse optional TLV tail after one BLE entry row; rewinds when bytes belong to the next entry.
pub fn parse_ble_entry_tlv(b: &[u8], i: &mut usize, more_entries_follow: bool) -> ScanEntryExtras {
    if *i >= b.len() {
        return ScanEntryExtras::default();
    }
    let start = *i;
    let extras = parse_scan_entry_tlv(b, i);
    if more_entries_follow && !looks_like_ble_entry_start(b, *i) {
        *i = start;
        return ScanEntryExtras::default();
    }
    extras
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_rcoi_and_cod() {
        let extras = ScanEntryExtras {
            rcoi_ie_payload: vec![0x03, 0x55, 0x5A, 0x03, 0xBA],
            ble_cod: Some([0x18, 0x04, 0x00]),
        };
        let mut wire = Vec::new();
        pack_scan_entry_tlv(&extras, &mut wire);
        let mut i = 0usize;
        let back = parse_scan_entry_tlv(&wire, &mut i);
        assert_eq!(back, extras);
        assert_eq!(i, wire.len());
    }

    #[test]
    fn empty_extras_pack_nothing() {
        let mut wire = Vec::new();
        pack_scan_entry_tlv(&ScanEntryExtras::default(), &mut wire);
        assert!(wire.is_empty());
    }
}
