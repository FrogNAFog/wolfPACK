//! Parse 802.11 management frames (beacon / probe response) into [`ApEntryV1`].
//!
//! Host-testable: no ESP-IDF types.
//!
//! LPR-camera wildcard probe IE fingerprinting (WiFi methods 2/3) lives in
//! [`wardriver_core::lpr_camera_probe_ie`] and is invoked from the promisc path.

use heapless::Vec as HVec;

use wardriver_proto::{ApEntryV1, AuthMini};

const WLAN_FC_TYPE_MASK: u16 = 0x000C;
const WLAN_FC_TYPE_MGMT: u16 = 0x0000;
const WLAN_FC_STYPE_MASK: u16 = 0x00F0;
const WLAN_FC_STYPE_BEACON: u16 = 0x0080;
const WLAN_FC_STYPE_PROBE_REQ: u16 = 0x0040;
const WLAN_FC_STYPE_PROBE_RESP: u16 = 0x0050;

const WLAN_CAP_ESS: u16 = 0x0001;
const WLAN_CAP_IBSS: u16 = 0x0002;
const WLAN_CAP_PRIVACY: u16 = 0x0010;

/// IE IDs we care about.
const IE_SSID: u8 = 0;
const IE_DSPARAMS: u8 = 3;
const IE_RSN: u8 = 48;
const IE_VENDOR: u8 = 221;
const IE_ROAMING_CONSORTIUM: u8 = 111;

const WPA_OUI: [u8; 4] = [0x00, 0x50, 0xf2, 0x01];

/// RSN AKM suite selectors (last byte of 00:0f:ac:X).
const RSN_AKM_8021X: u8 = 1;
const RSN_AKM_PSK: u8 = 2;
const RSN_AKM_SAE: u8 = 8;

/// If `mpdu` ends with a 32-bit FCS, strip it when tagged IEs would otherwise overrun.
fn trim_fcs_if_needed(mpdu: &[u8]) -> &[u8] {
    if mpdu.len() > 4 {
        let without = &mpdu[..mpdu.len() - 4];
        if beacon_ies_parseable(without) {
            return without;
        }
    }
    mpdu
}

fn beacon_ies_parseable(mpdu: &[u8]) -> bool {
    if mpdu.len() < 36 {
        return false;
    }
    let mut i = 36usize;
    let buf = mpdu;
    while i + 2 <= buf.len() {
        let elen = buf[i + 1] as usize;
        if i + 2 + elen > buf.len() {
            return false;
        }
        i += 2 + elen;
    }
    true
}

/// True if `mpdu` is an 802.11 **management probe request** (subtype 4).
pub fn is_mgmt_probe_request(mpdu: &[u8]) -> bool {
    let mpdu = trim_fcs_if_needed(mpdu);
    if mpdu.len() < 2 {
        return false;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    (fc & WLAN_FC_TYPE_MASK) == WLAN_FC_TYPE_MGMT
        && (fc & WLAN_FC_STYPE_MASK) == WLAN_FC_STYPE_PROBE_REQ
}

/// Extract AP metadata from a captured management MPDU (beacon or probe response).
///
/// `rx_channel` is used when the DSSS/HT channel IE is missing.
pub fn try_ap_from_mgmt_mpdu(mpdu: &[u8], rssi: i8, rx_channel: u8) -> Option<ApEntryV1> {
    let mpdu = trim_fcs_if_needed(mpdu);
    if mpdu.len() < 36 {
        return None;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    if fc & WLAN_FC_TYPE_MASK != WLAN_FC_TYPE_MGMT {
        return None;
    }
    let st = fc & WLAN_FC_STYPE_MASK;
    if st != WLAN_FC_STYPE_BEACON && st != WLAN_FC_STYPE_PROBE_RESP {
        return None;
    }

    // IBSS beacons are not infrastructure APs.
    let cap = u16::from_le_bytes([mpdu[34], mpdu[35]]);
    if (cap & WLAN_CAP_IBSS) != 0 && (cap & WLAN_CAP_ESS) == 0 {
        return None;
    }

    let mut bssid = [0u8; 6];
    bssid.copy_from_slice(&mpdu[16..22]);

    let privacy = (cap & WLAN_CAP_PRIVACY) != 0;
    let mut ssid_buf = [0u8; 32];
    let mut ssid_len: u8 = 0;
    let mut ch = rx_channel;
    let mut has_rsn = false;
    let mut has_wpa_ie = false;
    let mut akm: HVec<u8, 8> = HVec::new();

    let mut i = 36usize;
    while i + 2 <= mpdu.len() {
        let id = mpdu[i];
        let elen = mpdu[i + 1] as usize;
        i += 2;
        if i + elen > mpdu.len() {
            break;
        }
        let body = &mpdu[i..i + elen];
        match id {
            IE_SSID => {
                let n = elen.min(32);
                ssid_buf[..n].copy_from_slice(&body[..n]);
                ssid_len = n as u8;
            }
            IE_DSPARAMS => {
                if elen >= 1 {
                    ch = body[0];
                }
            }
            IE_RSN => {
                has_rsn = true;
                let _ = parse_rsn_akms(body, &mut akm);
            }
            IE_VENDOR if elen >= 4 && body[..4] == WPA_OUI => {
                has_wpa_ie = true;
            }
            _ => {}
        }
        i += elen;
    }

    let auth = classify_auth(privacy, has_rsn, has_wpa_ie, akm.as_slice());

    Some(ApEntryV1 {
        bssid,
        channel: ch,
        rssi,
        auth,
        ssid_len,
        ssid: ssid_buf,
        obs_mono_ms: 0,
    })
}

/// Like [`try_probe_req_detail_from_mgmt_mpdu`] but returns SSID bytes for **non-wildcard** probes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeReqDetail {
    pub src: [u8; 6],
    pub is_wildcard_ssid: bool,
    pub ssid_len: u8,
    pub ssid: [u8; 32],
}

/// Extract probe request source, wildcard bit, and SSID (if present) for Marauder-style passive checks.
pub fn try_probe_req_detail_from_mgmt_mpdu(
    mpdu: &[u8],
    _rssi: i8,
    _rx_channel: u8,
) -> Option<ProbeReqDetail> {
    if mpdu.len() < 24 {
        return None;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    if fc & WLAN_FC_TYPE_MASK != WLAN_FC_TYPE_MGMT {
        return None;
    }
    let st = fc & WLAN_FC_STYPE_MASK;
    if st != WLAN_FC_STYPE_PROBE_REQ {
        return None;
    }

    let mut src = [0u8; 6];
    src.copy_from_slice(&mpdu[10..16]);

    let mut i = 24usize;
    let mut saw_ssid = false;
    let mut wildcard = false;
    let mut ssid_buf = [0u8; 32];
    let mut ssid_len: u8 = 0;
    while i + 2 <= mpdu.len() {
        let id = mpdu[i];
        let elen = mpdu[i + 1] as usize;
        i += 2;
        if i + elen > mpdu.len() {
            break;
        }
        if id == IE_SSID {
            saw_ssid = true;
            wildcard = elen == 0;
            let n = elen.min(32);
            ssid_buf[..n].copy_from_slice(&mpdu[i..i + n]);
            ssid_len = n as u8;
            break;
        }
        i += elen;
    }
    if !saw_ssid {
        return None;
    }
    Some(ProbeReqDetail {
        src,
        is_wildcard_ssid: wildcard,
        ssid_len,
        ssid: ssid_buf,
    })
}

fn parse_rsn_akms(rsn: &[u8], out: &mut HVec<u8, 8>) -> Result<(), ()> {
    // RSN IE: version (2), group cipher (4), ptk count (2), ptk list, akm count (2), akm list
    if rsn.len() < 2 {
        return Ok(());
    }
    let mut o = 2usize;
    if o + 4 > rsn.len() {
        return Ok(());
    }
    o += 4; // group data cipher
    if o + 2 > rsn.len() {
        return Ok(());
    }
    let n_ptk = u16::from_le_bytes([rsn[o], rsn[o + 1]]) as usize;
    o += 2;
    o = o.saturating_add(n_ptk.saturating_mul(4));
    if o + 2 > rsn.len() {
        return Ok(());
    }
    let n_akm = u16::from_le_bytes([rsn[o], rsn[o + 1]]) as usize;
    o += 2;
    for _ in 0..n_akm {
        if o + 4 > rsn.len() {
            break;
        }
        let suite = &rsn[o..o + 4];
        o += 4;
        if suite[..3] == [0x00, 0x0f, 0xac] {
            if out.push(suite[3]).is_err() {
                break;
            }
        }
    }
    Ok(())
}

/// Extract Roaming Consortium IE (111) **body** from a beacon/probe-response MPDU.
#[must_use]
pub fn roaming_consortium_ie_payload_from_mgmt_mpdu(mpdu: &[u8]) -> Option<Vec<u8>> {
    let mpdu = trim_fcs_if_needed(mpdu);
    if mpdu.len() < 36 {
        return None;
    }
    let fc = u16::from_le_bytes([mpdu[0], mpdu[1]]);
    if fc & WLAN_FC_TYPE_MASK != WLAN_FC_TYPE_MGMT {
        return None;
    }
    let st = fc & WLAN_FC_STYPE_MASK;
    if st != WLAN_FC_STYPE_BEACON && st != WLAN_FC_STYPE_PROBE_RESP {
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
        if id == IE_ROAMING_CONSORTIUM {
            return Some(mpdu[i..i + elen].to_vec());
        }
        i += elen;
    }
    None
}

fn classify_auth(privacy: bool, has_rsn: bool, has_wpa_ie: bool, akm: &[u8]) -> AuthMini {
    let has_psk = akm.contains(&RSN_AKM_PSK);
    let has_sae = akm.contains(&RSN_AKM_SAE);
    let has_1x = akm.contains(&RSN_AKM_8021X);

    if has_rsn {
        if has_1x && !has_psk && !has_sae {
            return AuthMini::Wpa2Ent;
        }
        if has_psk && has_sae {
            return AuthMini::Wpa2Wpa3;
        }
        if has_sae && !has_psk {
            return AuthMini::Wpa3;
        }
        if has_psk {
            if has_wpa_ie {
                return AuthMini::WpaWpa2;
            }
            return AuthMini::Wpa2;
        }
        if privacy {
            return AuthMini::Wpa2;
        }
        return AuthMini::Open;
    }

    if has_wpa_ie {
        return AuthMini::Wpa;
    }

    if privacy {
        return AuthMini::Wep;
    }
    AuthMini::Open
}
