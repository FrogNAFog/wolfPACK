//! Golden 802.11 management MPDU fixtures (beacon + probe request).

use wardriver_core::ieee80211_parse::{try_ap_from_mgmt_mpdu, try_probe_req_detail_from_mgmt_mpdu};
use wardriver_core::wigle::{channel_to_frequency_mhz, format_bssid, ssid_bytes_to_wigle_string};
use wardriver_proto::AuthMini;

fn assert_beacon(
    mpdu: &[u8],
    rx_channel: u8,
    expect_bssid: &str,
    expect_ssid: &str,
    expect_channel: u8,
    expect_rssi: i8,
    expect_auth: AuthMini,
) {
    let ap = try_ap_from_mgmt_mpdu(mpdu, expect_rssi, rx_channel).expect("parse beacon mpdu");
    assert_eq!(format_bssid(&ap.bssid), expect_bssid);
    assert_eq!(
        ssid_bytes_to_wigle_string(&ap.ssid, ap.ssid_len as usize),
        expect_ssid
    );
    assert_eq!(ap.channel, expect_channel);
    assert_eq!(ap.rssi, expect_rssi);
    assert_eq!(ap.auth, expect_auth);
}

#[test]
fn golden_beacon_open_wpa2_ch6() {
    let mpdu = include_bytes!("fixtures/ieee80211/beacon_open_wpa2_ch6.bin");
    assert_beacon(
        mpdu,
        6,
        "aa:bb:cc:dd:ee:ff",
        "Snoop",
        6,
        -45,
        AuthMini::Wpa2,
    );
    assert_eq!(channel_to_frequency_mhz(6), 2437);
}

#[test]
fn golden_beacon_wpa2_ch36() {
    let mpdu = include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin");
    assert_beacon(
        mpdu,
        36,
        "aa:bb:cc:dd:ee:36",
        "FiveG-Test",
        36,
        -62,
        AuthMini::Wpa2,
    );
    assert_eq!(channel_to_frequency_mhz(36), 5180);
}

#[test]
fn golden_beacon_wpa2_ch149() {
    let mpdu = include_bytes!("fixtures/ieee80211/beacon_wpa2_ch149.bin");
    assert_beacon(
        mpdu,
        149,
        "aa:bb:cc:dd:ee:95",
        "FiveG-High",
        149,
        -58,
        AuthMini::Wpa2,
    );
    assert_eq!(channel_to_frequency_mhz(149), 5745);
}

#[test]
fn golden_probe_req_wildcard() {
    let mpdu = include_bytes!("fixtures/ieee80211/probe_req_wildcard.bin");
    let detail = try_probe_req_detail_from_mgmt_mpdu(mpdu, -70, 11).expect("probe detail");
    assert!(detail.is_wildcard_ssid);
    assert_eq!(detail.src, [0x70, 0xc9, 0x4e, 0x11, 0x22, 0x33]);
}

// ── Malformed / truncated frames ───────────────────────────────

#[test]
fn mpdu_too_short_for_mgmt_header() {
    assert!(try_ap_from_mgmt_mpdu(&[0x00, 0x00], -50, 6).is_none());
}

#[test]
fn mpdu_zero_length() {
    assert!(try_ap_from_mgmt_mpdu(&[], -50, 6).is_none());
}

#[test]
fn mpdu_truncated_at_ie_offset() {
    // Valid 36-byte mgmt header, then an IE with length that overruns
    let mut buf = [0u8; 40];
    buf[0] = 0x80;
    buf[1] = 0x00; // mgmt beacon
                   // BSSID at offset 16 — all zeros
                   // IE at offset 36: id=0 (SSID), len=100 → overruns
    buf[36] = 0x00;
    buf[37] = 100;
    let result = try_ap_from_mgmt_mpdu(&buf, -50, 6);
    // The parser breaks out of IE loop when len overruns, so it should still return
    // an AP entry with the fields it could parse
    assert!(result.is_some());
    assert_eq!(result.unwrap().ssid_len, 0);
}

#[test]
fn non_mgmt_subtype_rejected() {
    // Data frame (type=2, subtype=0)
    let mut buf = [0u8; 50];
    buf[0] = 0x08;
    buf[1] = 0x00;
    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());

    // Action frame (type=0, subtype=13 = 0xD0)
    buf[0] = 0xD0;
    buf[1] = 0x00;
    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());
}

#[test]
fn deauth_frame_rejected() {
    // Deauth = mgmt type, subtype 0xC0
    let mut buf = [0u8; 50];
    buf[0] = 0xC0;
    buf[1] = 0x00;
    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());
}

#[test]
fn disassoc_frame_rejected() {
    // Disassoc = mgmt type, subtype 0xA0
    let mut buf = [0u8; 50];
    buf[0] = 0xA0;
    buf[1] = 0x00;
    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());
}

#[test]
fn association_response_parsed() {
    // Assoc Resp = mgmt subtype 0x90 — not beacon or probe resp, so rejected
    let mut buf = [0u8; 50];
    buf[0] = 0x90;
    buf[1] = 0x00;
    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());
}

// ── Hidden SSID (zero-length) ──────────────────────────────────

#[test]
fn beacon_with_zero_length_ssid() {
    let mut buf = [0u8; 41];
    buf[0] = 0x80;
    buf[1] = 0x00; // beacon
    buf[16] = 0x11;
    buf[17] = 0x22;
    buf[18] = 0x33;
    buf[19] = 0x44;
    buf[20] = 0x55;
    buf[21] = 0x66;
    // Capability: ESS, no privacy
    buf[34] = 0x01;
    buf[35] = 0x00;
    // SSID IE: id=0, len=0
    buf[36] = 0x00;
    buf[37] = 0x00;
    // DSSS param: id=3, len=1, ch=6
    buf[38] = 0x03;
    buf[39] = 0x01;
    buf[40] = 0x06;

    let ap = try_ap_from_mgmt_mpdu(&buf, -55, 6).expect("parse");
    assert_eq!(ap.ssid_len, 0);
    assert_eq!(ap.auth, AuthMini::Open);
}

// ── Vendor IE with OUI but no payload ──────────────────────────

#[test]
fn vendor_ie_wpa_oui_with_no_payload() {
    let mut buf = [0u8; 51];
    buf[0] = 0x80;
    buf[1] = 0x00; // beacon
                   // BSSID
    buf[16] = 0xAA;
    buf[17] = 0xBB;
    // Cap: privacy bit set
    buf[34] = 0x10;
    buf[35] = 0x00;
    // SSID
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Test");
    // Vendor IE: WPA OUI (00:50:f2:01) with zero-length payload
    buf[42] = 0xDD; // vendor
    buf[43] = 0x04; // len=4
    buf[44] = 0x00;
    buf[45] = 0x50;
    buf[46] = 0xf2;
    buf[47] = 0x01;
    // DSSS
    buf[48] = 0x03;
    buf[49] = 0x01;
    buf[50] = 0x06;

    let ap = try_ap_from_mgmt_mpdu(&buf, -60, 6).expect("parse");
    // WPA OUI detected but no body → has_wpa_ie = true, no RSN → classified as WPA
    assert_eq!(ap.auth, AuthMini::Wpa);
}

// ── Probe request edge cases ───────────────────────────────────

#[test]
fn probe_request_with_named_ssid() {
    let mut buf = [0u8; 50];
    buf[0] = 0x40;
    buf[1] = 0x00; // probe request
                   // Source MAC (offset 10)
    buf[10] = 0xAA;
    buf[11] = 0xBB;
    buf[12] = 0xCC;
    buf[13] = 0xDD;
    buf[14] = 0xEE;
    buf[15] = 0xFF;
    // SSID IE at offset 24
    buf[24] = 0x00;
    buf[25] = 0x05;
    buf[26..31].copy_from_slice(b"Home1");

    let detail = try_probe_req_detail_from_mgmt_mpdu(&buf[..31], -50, 6).expect("probe");
    assert!(!detail.is_wildcard_ssid);
    assert_eq!(detail.ssid_len, 5);
    assert_eq!(&detail.ssid[..5], b"Home1");
    assert_eq!(detail.src, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
}

#[test]
fn probe_request_wildcard_ssid() {
    let mut buf = [0u8; 30];
    buf[0] = 0x40;
    buf[1] = 0x00; // probe request
    buf[10] = 0x11;
    buf[11] = 0x22; // src MAC
    buf[24] = 0x00;
    buf[25] = 0x00; // SSID len=0

    let detail = try_probe_req_detail_from_mgmt_mpdu(&buf, -70, 11).expect("probe");
    assert!(detail.is_wildcard_ssid);
    assert_eq!(detail.ssid_len, 0);
}

#[test]
fn probe_request_no_ssid_ie() {
    let mut buf = [0u8; 30];
    buf[0] = 0x40;
    buf[1] = 0x00;
    // No SSID IE in IEs (offset 24)
    buf[24] = 0x03;
    buf[25] = 0x01;
    buf[26] = 0x06; // only DSSS param

    assert!(try_probe_req_detail_from_mgmt_mpdu(&buf[..27], -70, 11).is_none());
}

// ── Authentication mode coverage ───────────────────────────────

#[test]
fn beacon_with_wpa3_sae_auth() {
    let mut buf = [0u8; 80];
    buf[0] = 0x80;
    buf[1] = 0x00; // beacon
    buf[16] = 0x01; // BSSID
    buf[34] = 0x10;
    buf[35] = 0x00; // privacy=1

    // SSID
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"WiFi");

    // RSN IE with SAE AKM only (00:0f:ac:08)
    buf[42] = 0x30; // RSN id
    buf[43] = 18; // len
                  // version
    buf[44] = 0x01;
    buf[45] = 0x00;
    // group cipher
    buf[46] = 0x00;
    buf[47] = 0x0f;
    buf[48] = 0xac;
    buf[49] = 0x04;
    // ptk count=1, ptk
    buf[50] = 0x01;
    buf[51] = 0x00;
    buf[52] = 0x00;
    buf[53] = 0x0f;
    buf[54] = 0xac;
    buf[55] = 0x08;
    // akm count=1
    buf[56] = 0x01;
    buf[57] = 0x00;
    // SAE AKM
    buf[58] = 0x00;
    buf[59] = 0x0f;
    buf[60] = 0xac;
    buf[61] = 0x08;

    let ap = try_ap_from_mgmt_mpdu(&buf[..62], -55, 6).expect("parse");
    assert_eq!(ap.auth, AuthMini::Wpa3);
}

#[test]
fn beacon_with_wpa3_enterprise() {
    let mut buf = [0u8; 80];
    buf[0] = 0x80;
    buf[1] = 0x00;
    buf[16] = 0x02;
    buf[34] = 0x10;
    buf[35] = 0x00;
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Corp");

    // RSN with 802.1X AKM only
    buf[42] = 0x30;
    buf[43] = 18;
    buf[44] = 0x01;
    buf[45] = 0x00; // version
    buf[46] = 0x00;
    buf[47] = 0x0f;
    buf[48] = 0xac;
    buf[49] = 0x04; // group
    buf[50] = 0x01;
    buf[51] = 0x00; // ptk count
    buf[52] = 0x00;
    buf[53] = 0x0f;
    buf[54] = 0xac;
    buf[55] = 0x08;
    buf[56] = 0x01;
    buf[57] = 0x00; // akm count
    buf[58] = 0x00;
    buf[59] = 0x0f;
    buf[60] = 0xac;
    buf[61] = 0x01; // 802.1X

    let ap = try_ap_from_mgmt_mpdu(&buf[..62], -50, 6).expect("parse");
    assert_eq!(ap.auth, AuthMini::Wpa2Ent);
}

#[test]
fn beacon_with_wpa2_wpa3_transition() {
    let mut buf = [0u8; 80];
    buf[0] = 0x80;
    buf[1] = 0x00;
    buf[16] = 0x03;
    buf[34] = 0x10;
    buf[35] = 0x00;
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Dual");

    // RSN with both PSK (02) and SAE (08) AKMs
    buf[42] = 0x30;
    buf[43] = 26;
    buf[44] = 0x01;
    buf[45] = 0x00;
    buf[46] = 0x00;
    buf[47] = 0x0f;
    buf[48] = 0xac;
    buf[49] = 0x04;
    buf[50] = 0x02;
    buf[51] = 0x00; // ptk count=2
    buf[52] = 0x00;
    buf[53] = 0x0f;
    buf[54] = 0xac;
    buf[55] = 0x08;
    buf[56] = 0x00;
    buf[57] = 0x0f;
    buf[58] = 0xac;
    buf[59] = 0x04;
    buf[60] = 0x02;
    buf[61] = 0x00; // akm count=2
    buf[62] = 0x00;
    buf[63] = 0x0f;
    buf[64] = 0xac;
    buf[65] = 0x02; // PSK
    buf[66] = 0x00;
    buf[67] = 0x0f;
    buf[68] = 0xac;
    buf[69] = 0x08; // SAE

    let ap = try_ap_from_mgmt_mpdu(&buf[..70], -55, 6).expect("parse");
    assert_eq!(ap.auth, AuthMini::Wpa2Wpa3);
}

#[test]
fn beacon_with_wep_only() {
    let mut buf = [0u8; 42];
    buf[0] = 0x80;
    buf[1] = 0x00;
    buf[16] = 0x04;
    buf[34] = 0x10;
    buf[35] = 0x00; // privacy=1, no WPA/RSN
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Old!");

    let ap = try_ap_from_mgmt_mpdu(&buf, -70, 6).expect("parse");
    assert_eq!(ap.auth, AuthMini::Wep);
}

// ── Channel edge cases ─────────────────────────────────────────

#[test]
fn probe_response_parsed_like_beacon() {
    let mut buf = [0u8; 45];
    buf[0] = 0x50;
    buf[1] = 0x00; // probe response subtype
    buf[16] = 0x05;
    buf[34] = 0x01;
    buf[35] = 0x00;
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Resp");
    buf[42] = 0x03;
    buf[43] = 0x01;
    buf[44] = 0x06;

    let ap = try_ap_from_mgmt_mpdu(&buf, -60, 6).expect("parse");
    assert_eq!(ap.channel, 6);
}

// ── IBSS rejection ─────────────────────────────────────────────

#[test]
fn ibss_beacon_rejected() {
    let mut buf = [0u8; 40];
    buf[0] = 0x80;
    buf[1] = 0x00;
    buf[16] = 0x06;
    // IBSS bit set, ESS bit clear
    buf[34] = 0x02;
    buf[35] = 0x00;
    buf[36] = 0x00;
    buf[37] = 0x00; // empty SSID

    assert!(try_ap_from_mgmt_mpdu(&buf, -50, 6).is_none());
}

#[test]
fn ibss_with_ess_bit_parsed() {
    let mut buf = [0u8; 42];
    buf[0] = 0x80;
    buf[1] = 0x00;
    buf[16] = 0x07;
    // Both IBSS and ESS set — treated as infrastructure
    buf[34] = 0x03;
    buf[35] = 0x00;
    buf[36] = 0x00;
    buf[37] = 0x04;
    buf[38..42].copy_from_slice(b"Both");

    let ap = try_ap_from_mgmt_mpdu(&buf, -55, 6).expect("parse");
    assert_eq!(ap.auth, AuthMini::Open);
}
