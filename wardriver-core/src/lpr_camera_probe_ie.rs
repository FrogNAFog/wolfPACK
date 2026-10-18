//! Wildcard probe-request IE matcher for LPR-camera primary fingerprints.
//!
//! Matches production LPR-camera probes: wildcard SSID, arbitrary prefix IEs, then a fixed
//! **suffix** from the Lite-On vendor IE through WPA vendor (`506f9a…` → HT → VHT → `0050f2…`).
//! This mirrors the legacy string matcher’s canonicalize step (anchor at Lite-On, ignore prefix tags).

const WLAN_FC_TYPE_MASK: u16 = 0x000C;
const WLAN_FC_TYPE_MGMT: u16 = 0x0000;
const WLAN_FC_STYPE_MASK: u16 = 0x00F0;
const WLAN_FC_STYPE_PROBE_REQ: u16 = 0x0040;

/// Byte offset of the first IE in a probe-request management MPDU (24-byte 802.11 mgmt header).
const PROBE_REQ_BODY_OFFSET: usize = 24;

const IE_SSID: u8 = 0;
const IE_VENDOR: u8 = 221;

const LPR_VENDOR_LITEON: [u8; 7] = [0x50, 0x6f, 0x9a, 0x16, 0x03, 0x01, 0x03];
const LPR_VENDOR_WPA: [u8; 7] = [0x00, 0x50, 0xf2, 0x08, 0x00, 0x00, 0x00];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpectedIe {
    TagOnly(u8),
    VendorPayload([u8; 7]),
}

/// Ordered tail after the Lite-On anchor (tag IDs only for HT/VHT; any element length).
const LPR_LITEON_SUFFIX: &[ExpectedIe] = &[
    ExpectedIe::VendorPayload(LPR_VENDOR_LITEON),
    ExpectedIe::TagOnly(45),
    ExpectedIe::TagOnly(191),
    ExpectedIe::VendorPayload(LPR_VENDOR_WPA),
];

fn tlv_fits(i: usize, elen: usize, len: usize) -> bool {
    i + 2 + elen <= len
}

fn coalesce_wildcard_ssid_headers(body: &[u8], mut i: usize) -> usize {
    while i + 2 <= body.len() && body[i] == 0 && body[i + 1] == 0 {
        i += 2;
    }
    i
}

/// Walk TLVs from `body[0]`; `true` when the full slice matches `expected` and is consumed.
fn match_lpr_camera_probe_ies(body: &[u8], expected: &[ExpectedIe]) -> bool {
    if body.is_empty() {
        return false;
    }
    let mut i = 0usize;
    for exp in expected {
        if i + 2 > body.len() {
            return false;
        }
        let id = body[i];
        let elen = body[i + 1] as usize;
        if !tlv_fits(i, elen, body.len()) {
            return false;
        }

        match exp {
            ExpectedIe::TagOnly(tag) => {
                if id != *tag {
                    return false;
                }
                i += 2 + elen;
            }
            ExpectedIe::VendorPayload(vendor) => {
                if id != IE_VENDOR || elen != vendor.len() {
                    return false;
                }
                if body[i + 2..i + 2 + elen] != vendor[..] {
                    return false;
                }
                i += 2 + elen;
            }
        }
    }
    i == body.len()
}

fn is_liteon_vendor_tlv(body: &[u8], i: usize) -> bool {
    if i + 2 > body.len() {
        return false;
    }
    let elen = body[i + 1] as usize;
    body[i] == IE_VENDOR
        && elen == LPR_VENDOR_LITEON.len()
        && tlv_fits(i, elen, body.len())
        && body[i + 2..i + 2 + elen] == LPR_VENDOR_LITEON[..]
}

/// Wildcard SSID at the start, optional prefix IEs, then Lite-On → HT → VHT → WPA to end.
fn match_primary_lpr_camera_probe_ies(body: &[u8]) -> bool {
    if body.len() < 2 || body[0] != IE_SSID || body[1] != 0 {
        return false;
    }
    let mut i = coalesce_wildcard_ssid_headers(body, 2);
    while i + 2 <= body.len() {
        let elen = body[i + 1] as usize;
        if !tlv_fits(i, elen, body.len()) {
            return false;
        }
        if is_liteon_vendor_tlv(body, i)
            && match_lpr_camera_probe_ies(&body[i..], LPR_LITEON_SUFFIX)
        {
            return true;
        }
        i += 2 + elen;
    }
    false
}

/// Match primary signature; retry with 4-byte FCS stripped when body length includes a trailer.
#[must_use]
pub fn probe_body_matches_lpr_camera_primary(body: &[u8]) -> bool {
    if body.len() < 2 {
        return false;
    }
    if match_primary_lpr_camera_probe_ies(body) {
        return true;
    }
    if body.len() > 4 && match_primary_lpr_camera_probe_ies(&body[..body.len() - 4]) {
        return true;
    }
    false
}

/// `true` when `mpdu` is a probe request whose IE body matches the primary LPR-camera sequence.
#[must_use]
pub fn probe_req_mpdu_matches_lpr_camera_primary(mpdu: &[u8]) -> bool {
    if mpdu.len() < PROBE_REQ_BODY_OFFSET {
        return false;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    if fc & WLAN_FC_TYPE_MASK != WLAN_FC_TYPE_MGMT {
        return false;
    }
    let st = fc & WLAN_FC_STYPE_MASK;
    if st != WLAN_FC_STYPE_PROBE_REQ {
        return false;
    }
    probe_body_matches_lpr_camera_primary(&mpdu[PROBE_REQ_BODY_OFFSET..])
}
