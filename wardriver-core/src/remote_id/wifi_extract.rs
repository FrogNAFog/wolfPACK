//! Extract OpenDroneID payloads from 802.11 management MPDUs.

use crate::remote_id::types::RemoteIdWifiTransport;

const WLAN_FC_TYPE_MASK: u16 = 0x000C;
const WLAN_FC_TYPE_MGMT: u16 = 0x0000;
const WLAN_FC_STYPE_MASK: u16 = 0x00F0;
const WLAN_FC_STYPE_BEACON: u16 = 0x0080;
const WLAN_FC_STYPE_ACTION: u16 = 0x00D0;

const IE_VENDOR: u8 = 221;

const ASTM_OUI: [u8; 3] = [0xFA, 0x0B, 0xBC];
const PARROT_OUI: [u8; 3] = [0x90, 0x3A, 0xE6];
const ODID_APP_CODE: u8 = 0x0D;

const NAN_DEST_MAC: [u8; 6] = [0x51, 0x6F, 0x9A, 0x01, 0x00, 0x00];
const NAN_OUI: [u8; 3] = [0x50, 0x6F, 0x9A];
const NAN_OUI_TYPE: u8 = 0x13;
const NAN_CATEGORY: u8 = 0x04;
const NAN_ACTION_CODE: u8 = 0x09;
const NAN_SERVICE_ID: [u8; 6] = [0x88, 0x69, 0x19, 0x9D, 0x92, 0x09];

/// Locate an OpenDroneID payload in a captured management MPDU.
#[must_use]
pub fn extract_wifi_remote_id_from_mgmt(mpdu: &[u8]) -> Option<(&[u8], RemoteIdWifiTransport)> {
    let mpdu = trim_fcs_if_needed(mpdu);
    if mpdu.len() < 24 {
        return None;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    if fc & WLAN_FC_TYPE_MASK != WLAN_FC_TYPE_MGMT {
        return None;
    }
    let st = fc & WLAN_FC_STYPE_MASK;
    if st == WLAN_FC_STYPE_BEACON {
        return extract_from_beacon(mpdu).map(|p| (p, RemoteIdWifiTransport::Beacon));
    }
    if st == WLAN_FC_STYPE_ACTION {
        return extract_from_nan_action(mpdu).map(|p| (p, RemoteIdWifiTransport::Nan));
    }
    None
}

fn trim_fcs_if_needed(mpdu: &[u8]) -> &[u8] {
    if mpdu.len() > 4 {
        let without = &mpdu[..mpdu.len() - 4];
        if ies_parseable(without) {
            return without;
        }
    }
    mpdu
}

fn ies_parseable(mpdu: &[u8]) -> bool {
    if mpdu.len() < 36 {
        return false;
    }
    let mut i = 36usize;
    while i + 2 <= mpdu.len() {
        let elen = mpdu[i + 1] as usize;
        if i + 2 + elen > mpdu.len() {
            return false;
        }
        i += 2 + elen;
    }
    true
}

fn extract_from_beacon(mpdu: &[u8]) -> Option<&[u8]> {
    if mpdu.len() < 36 {
        return None;
    }
    let mut i = 36usize;
    while i + 2 <= mpdu.len() {
        let id = mpdu[i];
        let elen = mpdu[i + 1] as usize;
        i += 2;
        if i + elen > mpdu.len() {
            break;
        }
        let body = &mpdu[i..i + elen];
        if id == IE_VENDOR && elen >= 4 {
            if (body[..3] == ASTM_OUI || body[..3] == PARROT_OUI) && body[3] == ODID_APP_CODE {
                return body.get(4..);
            }
        }
        i += elen;
    }
    None
}

fn extract_from_nan_action(mpdu: &[u8]) -> Option<&[u8]> {
    if mpdu.len() < 24 {
        return None;
    }
    if mpdu[4..10] != NAN_DEST_MAC {
        return None;
    }

    let mut off = 24usize;
    if off + 6 > mpdu.len() {
        return None;
    }
    if mpdu[off] != NAN_CATEGORY
        || mpdu[off + 1] != NAN_ACTION_CODE
        || mpdu[off + 2..off + 5] != NAN_OUI
        || mpdu[off + 5] != NAN_OUI_TYPE
    {
        return None;
    }
    off += 6;

    // Service Descriptor attribute (id 0x03) with ODID service id.
    if off + 14 > mpdu.len() {
        return None;
    }
    if mpdu[off] != 0x03 {
        return None;
    }
    let attr_len = u16::from_le_bytes([mpdu[off + 1], mpdu[off + 2]]) as usize;
    if off + 3 + attr_len > mpdu.len() {
        return None;
    }
    if mpdu[off + 3..off + 9] != NAN_SERVICE_ID {
        return None;
    }
    let service_info_len = mpdu[off + 12] as usize;
    let service_info_off = off + 13;
    if service_info_off + service_info_len > mpdu.len() {
        return None;
    }
    // Skip the 1-byte ODID message counter; remainder is message pack / message.
    mpdu.get(service_info_off + 1..service_info_off + service_info_len)
}
