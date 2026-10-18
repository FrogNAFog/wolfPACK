//! Host utility: write `wardriver-core/tests/fixtures/ieee80211/*` golden MPDUs.
//!
//! Run from repo root:
//! `cargo run -p wardriver-tools --bin write_ieee80211_fixtures`

use std::path::PathBuf;

use wardriver_core::ieee80211_parse::try_ap_from_mgmt_mpdu;
use wardriver_proto::AuthMini;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = root.join("../wardriver-core/tests/fixtures/ieee80211");
    std::fs::create_dir_all(&out_dir).expect("mkdir ieee80211 fixtures");

    write_beacon_fixture(
        &out_dir.join("beacon_open_wpa2_ch6.bin"),
        &out_dir.join("beacon_open_wpa2_ch6.expect.json"),
        [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        "Snoop",
        6,
        true,
        -45,
    );
    write_beacon_fixture(
        &out_dir.join("beacon_wpa2_ch36.bin"),
        &out_dir.join("beacon_wpa2_ch36.expect.json"),
        [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x36],
        "FiveG-Test",
        36,
        true,
        -62,
    );
    write_beacon_fixture(
        &out_dir.join("beacon_wpa2_ch149.bin"),
        &out_dir.join("beacon_wpa2_ch149.expect.json"),
        [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0x95],
        "FiveG-High",
        149,
        true,
        -58,
    );

    let probe = build_probe_req_wildcard([0x70, 0xc9, 0x4e, 0x11, 0x22, 0x33]);
    std::fs::write(out_dir.join("probe_req_wildcard.bin"), &probe).expect("write probe");
    println!("wrote {}", out_dir.display());
}

fn write_beacon_fixture(
    bin_path: &PathBuf,
    json_path: &PathBuf,
    bssid: [u8; 6],
    ssid: &str,
    channel: u8,
    wpa2: bool,
    rssi: i8,
) {
    let mpdu = build_beacon_mpdu(bssid, ssid, channel, wpa2);
    std::fs::write(bin_path, &mpdu).expect("write beacon bin");
    let ap = try_ap_from_mgmt_mpdu(&mpdu, rssi, channel).expect("parse beacon");
    let json = format!(
        r#"{{
  "bssid": "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
  "ssid": "{ssid}",
  "channel": {channel},
  "rssi": {rssi},
  "auth": "{auth}"
}}"#,
        bssid[0],
        bssid[1],
        bssid[2],
        bssid[3],
        bssid[4],
        bssid[5],
        ssid = ssid,
        channel = ap.channel,
        rssi = ap.rssi,
        auth = auth_label(ap.auth),
    );
    std::fs::write(json_path, json).expect("write expect json");
}

fn auth_label(a: AuthMini) -> &'static str {
    match a {
        AuthMini::Open => "open",
        AuthMini::Wep => "wep",
        AuthMini::Wpa => "wpa",
        AuthMini::Wpa2 => "wpa2",
        AuthMini::WpaWpa2 => "wpa_wpa2",
        AuthMini::Wpa2Ent => "wpa2_ent",
        AuthMini::Wpa3 => "wpa3",
        AuthMini::Wpa2Wpa3 => "wpa2_wpa3",
        AuthMini::Unknown => "unknown",
    }
}

/// Minimal infrastructure beacon with SSID + DS channel (+ optional WPA2 RSN IE).
pub fn build_beacon_mpdu(bssid: [u8; 6], ssid: &str, channel: u8, wpa2: bool) -> Vec<u8> {
    let mut v = Vec::new();
    // Frame control: management beacon (subtype 8).
    v.push(0x80);
    v.push(0x00);
    v.extend_from_slice(&[0x00, 0x00]); // duration
    v.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]); // DA broadcast
    v.extend_from_slice(&bssid); // SA
    v.extend_from_slice(&bssid); // BSSID
    v.extend_from_slice(&[0x00, 0x00]); // sequence
    v.extend_from_slice(&[0u8; 8]); // timestamp
    v.extend_from_slice(&[0x64, 0x00]); // beacon interval 100 TU
    let cap: u16 = if wpa2 { 0x0011 } else { 0x0001 }; // ESS (+ privacy for WPA2)
    v.extend_from_slice(&cap.to_le_bytes());
    // SSID IE
    let ssid_bytes = ssid.as_bytes();
    v.push(0);
    v.push(ssid_bytes.len() as u8);
    v.extend_from_slice(ssid_bytes);
    // DS Parameter Set (primary channel)
    v.push(3);
    v.push(1);
    v.push(channel);
    if wpa2 {
        // RSN IE: WPA2-PSK-CCMP (minimal valid body for classify_auth).
        let rsn: [u8; 20] = [
            0x01, 0x00, 0x00, 0x0f, 0xac, 0x02, 0x02, 0x00, 0x00, 0x0f, 0xac, 0x04, 0x01, 0x00,
            0x00, 0x0f, 0xac, 0x02, 0x00, 0x00,
        ];
        v.push(48);
        v.push(rsn.len() as u8);
        v.extend_from_slice(&rsn);
    }
    v
}

/// Probe request with wildcard (empty) SSID IE.
pub fn build_probe_req_wildcard(src: [u8; 6]) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(0x40);
    v.push(0x00);
    v.extend_from_slice(&[0x00, 0x00]);
    v.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    v.extend_from_slice(&src);
    v.extend_from_slice(&src);
    v.extend_from_slice(&[0x00, 0x00]);
    // Wildcard SSID
    v.push(0);
    v.push(0);
    v
}
