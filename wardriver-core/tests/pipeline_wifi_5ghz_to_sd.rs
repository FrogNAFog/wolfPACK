//! End-to-end host pipeline: 5 GHz WiFi beacon → ESP-NOW ScanFrag → CSV line.
//!
//! Firmware-only stages (promisc RX, ESP-NOW TX, SD FAT write) are out of scope; this
//! test covers every host-testable step on the sniffer→control→encode path.

use std::collections::{HashMap, VecDeque};

use indexmap::IndexMap;
use wardriver_core::gps_anchor::{self, GpsFix, MAX_FIX_TIMELINE_DELTA_MS};
use wardriver_core::ieee80211_parse::{
    roaming_consortium_ie_payload_from_mgmt_mpdu, try_ap_from_mgmt_mpdu,
};
use wardriver_core::mac_table::{LaneFlags, MacTable};
use wardriver_core::scan_reasm::{ingest_wifi_scan_frag, ingest_wifi_scan_frag_with_extras};
use wardriver_core::wigle::{ap_entry_to_wifi_line, format_wifi_csv_line};
use wardriver_core::wigle_rcoi::rcois_from_ie_payload;
use wardriver_proto::{
    pack_scan_frag, pack_scan_frag_with_extras, unpack_scan_frag, unpack_scan_frag_with_extras,
    ApEntryV1, ScanEntryExtras, ScanFragV1Header, FRAG_FLAG_TRANSPORT_END,
};

const SCAN_END_UNIX_MS: u64 = 1_777_190_400_000; // 2026-04-26 08:00:00 UTC
const SCAN_END_MS: u32 = 50_000;
const OBS_MONO_MS: u32 = SCAN_END_MS;

fn sample_gps_hist() -> VecDeque<(i64, GpsFix)> {
    let mut hist = VecDeque::new();
    hist.push_back((
        SCAN_END_UNIX_MS as i64,
        GpsFix {
            utc_unix_ms: SCAN_END_UNIX_MS as i64,
            lat: 37.7749,
            lon: -122.4194,
            altitude_m: Some(12.0),
            hdop: Some(1.5),
            accuracy_m: None,
            fix_quality: 1,
        },
    ));
    hist
}

fn ap_from_beacon(beacon_bin: &'static [u8], rx_channel: u8, rssi: i8) -> ApEntryV1 {
    let mut ap = try_ap_from_mgmt_mpdu(beacon_bin, rssi, rx_channel).expect("beacon parse");
    ap.obs_mono_ms = OBS_MONO_MS;
    ap
}

fn scan_batch_hdr(frag_idx: u8, frag_total: u8, ap_count: u8) -> ScanFragV1Header {
    ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 3,
        seq: 10,
        scan_batch_id: 20,
        sniffer_boot_id: 0xAABB_CCDD,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx,
        frag_total,
        ap_count,
    }
}

/// Encode one AP through GPS match, mac_table, and WiGLE line (post-reassembly).
fn csv_line_for_ap(
    ap: &ApEntryV1,
    mac_table: &mut MacTable,
    hist: &VecDeque<(i64, GpsFix)>,
) -> String {
    let obs_unix =
        gps_anchor::observed_unix_ms_from_anchor(SCAN_END_MS, SCAN_END_UNIX_MS, ap.obs_mono_ms)
            .expect("obs unix");
    let fix = gps_anchor::match_gps_fix_for_obs(hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS)
        .expect("gps match");
    let allow = mac_table.allow(
        &ap.bssid,
        fix.lat,
        fix.lon,
        LaneFlags::WIGLE_WIFI,
        50.0,
        Some(ap.rssi),
    );
    assert!(allow.decision.allow_row);
    let ts = gps_anchor::utc_string_from_unix_ms(obs_unix).expect("utc string");
    let line_inp = ap_entry_to_wifi_line(
        ap,
        &ts,
        Some(fix.lat),
        Some(fix.lon),
        fix.altitude_m.map(|a| a.round() as i32),
        fix.hdop.map(gps_anchor::hdop_to_accuracy_m),
    );
    format_wifi_csv_line(&line_inp)
}

fn pipeline_wifi_line_for_channel(
    beacon_bin: &'static [u8],
    rx_channel: u8,
    rssi: i8,
    golden_line: &'static str,
) {
    // 1) Sniffer-side: promisc parse (host-testable substitute for wifi_promisc).
    let mut ap = try_ap_from_mgmt_mpdu(beacon_bin, rssi, rx_channel).expect("beacon parse");
    ap.obs_mono_ms = OBS_MONO_MS;

    // 2) Sniffer-side: pack ESP-NOW ScanFrag wire bytes.
    let hdr = ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 3,
        seq: 10,
        scan_batch_id: 20,
        sniffer_boot_id: 0xAABB_CCDD,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx: 0,
        frag_total: 1,
        ap_count: 1,
    };
    let wire = pack_scan_frag(hdr, std::slice::from_ref(&ap)).expect("pack scan frag");
    let (back_hdr, aps) = unpack_scan_frag(&wire).expect("unpack scan frag");
    assert_eq!(aps.len(), 1);

    // 3) Control-side: dedup + reassembly (single-frag batch).
    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();
    let (complete_hdr, complete_aps) =
        ingest_wifi_scan_frag(back_hdr, aps, &mut dedup, &mut reasm, 1_000)
            .expect("wifi reasm complete");
    assert_eq!(complete_aps.len(), 1);
    let ap = &complete_aps[0];

    // 4) GPS anchor + timeline match.
    let obs_unix = gps_anchor::observed_unix_ms_from_anchor(
        complete_hdr.sniffer_scan_end_ms,
        complete_hdr.sniffer_scan_end_unix_ms,
        ap.obs_mono_ms,
    )
    .expect("obs unix");
    let hist = sample_gps_hist();
    let fix = gps_anchor::match_gps_fix_for_obs(&hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS)
        .expect("gps match");

    // 5) WiGLE geo dedup (first row for this BSSID at this location).
    let mut mac_table = MacTable::new(64, 16);
    let allow = mac_table.allow(
        &ap.bssid,
        fix.lat,
        fix.lon,
        LaneFlags::WIGLE_WIFI,
        50.0,
        Some(ap.rssi),
    );
    assert!(allow.decision.allow_row);

    // 6) CSV encode (bytes that SD writer would enqueue).
    let ts = gps_anchor::utc_string_from_unix_ms(obs_unix).expect("utc string");
    let line_inp = ap_entry_to_wifi_line(
        ap,
        &ts,
        Some(fix.lat),
        Some(fix.lon),
        fix.altitude_m.map(|a| a.round() as i32),
        fix.hdop.map(gps_anchor::hdop_to_accuracy_m),
    );
    let line = format_wifi_csv_line(&line_inp);
    assert_eq!(line.trim(), golden_line.trim());
}

#[test]
fn pipeline_wifi_5ghz_ch36_to_csv_line() {
    pipeline_wifi_line_for_channel(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line"),
    );
}

#[test]
fn pipeline_wifi_5ghz_ch149_to_csv_line() {
    pipeline_wifi_line_for_channel(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch149.bin"),
        149,
        -58,
        include_str!("fixtures/wigle/wifi_row_5ghz_ch149.line"),
    );
}

#[test]
fn pipeline_wifi_2ghz_ch6_to_csv_line() {
    pipeline_wifi_line_for_channel(
        include_bytes!("fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
        include_str!("fixtures/wigle/wifi_row_minimal_pipeline.line"),
    );
}

#[test]
fn pipeline_5ghz_two_aps_same_frag_to_csv_lines() {
    // Two UNII APs (ch36 + ch149) packed into a single ScanFrag — common when the frame budget fits.
    let ap36 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let ap149 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch149.bin"),
        149,
        -58,
    );
    let aps = vec![ap36, ap149];

    let hdr = scan_batch_hdr(0, 1, 2);
    let wire = pack_scan_frag(hdr, &aps).expect("pack multi-ap frag");
    let (back_hdr, unpacked) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(unpacked.len(), 2);

    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();
    let (_complete_hdr, complete_aps) =
        ingest_wifi_scan_frag(back_hdr, unpacked, &mut dedup, &mut reasm, 1_000)
            .expect("reasm complete");
    assert_eq!(complete_aps.len(), 2);

    let hist = sample_gps_hist();
    let mut mac_table = MacTable::new(64, 16);
    let line36 = csv_line_for_ap(&complete_aps[0], &mut mac_table, &hist);
    let line149 = csv_line_for_ap(&complete_aps[1], &mut mac_table, &hist);
    assert_eq!(
        line36.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line").trim()
    );
    assert_eq!(
        line149.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch149.line").trim()
    );
}

#[test]
fn pipeline_5ghz_two_aps_two_frags_to_csv_lines() {
    // Same batch split across two fragments (one AP each) — wolfPACK multi-frame dump path.
    let ap36 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let ap149 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch149.bin"),
        149,
        -58,
    );

    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();

    let wire0 =
        pack_scan_frag(scan_batch_hdr(0, 2, 1), std::slice::from_ref(&ap36)).expect("pack frag 0");
    let (hdr0, aps0) = unpack_scan_frag(&wire0).expect("unpack frag 0");
    assert!(
        ingest_wifi_scan_frag(hdr0, aps0, &mut dedup, &mut reasm, 1_000).is_none(),
        "first frag incomplete"
    );

    let wire1 =
        pack_scan_frag(scan_batch_hdr(1, 2, 1), std::slice::from_ref(&ap149)).expect("pack frag 1");
    let (hdr1, aps1) = unpack_scan_frag(&wire1).expect("unpack frag 1");
    let (_complete_hdr, complete_aps) =
        ingest_wifi_scan_frag(hdr1, aps1, &mut dedup, &mut reasm, 1_001).expect("reasm complete");
    assert_eq!(complete_aps.len(), 2);
    assert_eq!(complete_aps[0].channel, 36);
    assert_eq!(complete_aps[1].channel, 149);

    let hist = sample_gps_hist();
    let mut mac_table = MacTable::new(64, 16);
    let line36 = csv_line_for_ap(&complete_aps[0], &mut mac_table, &hist);
    let line149 = csv_line_for_ap(&complete_aps[1], &mut mac_table, &hist);
    assert_eq!(
        line36.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line").trim()
    );
    assert_eq!(
        line149.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch149.line").trim()
    );
}

#[test]
fn pipeline_mixed_band_same_frag_to_csv_lines() {
    // 2.4 GHz (ch6) + 5 GHz (ch36) in one ScanFrag — dual-band sweep merged into one batch.
    let ap6 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let ap36 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let aps = vec![ap6, ap36];

    let hdr = scan_batch_hdr(0, 1, 2);
    let wire = pack_scan_frag(hdr, &aps).expect("pack mixed-band frag");
    let (back_hdr, unpacked) = unpack_scan_frag(&wire).expect("unpack");
    assert_eq!(unpacked.len(), 2);
    assert_eq!(unpacked[0].channel, 6);
    assert_eq!(unpacked[1].channel, 36);

    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();
    let (_complete_hdr, complete_aps) =
        ingest_wifi_scan_frag(back_hdr, unpacked, &mut dedup, &mut reasm, 1_000)
            .expect("reasm complete");
    assert_eq!(complete_aps.len(), 2);

    let hist = sample_gps_hist();
    let mut mac_table = MacTable::new(64, 16);
    let line6 = csv_line_for_ap(&complete_aps[0], &mut mac_table, &hist);
    let line36 = csv_line_for_ap(&complete_aps[1], &mut mac_table, &hist);
    assert_eq!(
        line6.trim(),
        include_str!("fixtures/wigle/wifi_row_minimal_pipeline.line").trim()
    );
    assert_eq!(
        line36.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line").trim()
    );
}

#[test]
fn pipeline_mixed_band_two_frags_to_csv_lines() {
    // Mixed-band batch split across fragments (ch6 in frag 0, ch36 in frag 1).
    let ap6 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let ap36 = ap_from_beacon(
        include_bytes!("fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );

    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();

    let wire0 =
        pack_scan_frag(scan_batch_hdr(0, 2, 1), std::slice::from_ref(&ap6)).expect("pack frag 0");
    let (hdr0, aps0) = unpack_scan_frag(&wire0).expect("unpack frag 0");
    assert!(
        ingest_wifi_scan_frag(hdr0, aps0, &mut dedup, &mut reasm, 1_000).is_none(),
        "first frag incomplete"
    );

    let wire1 =
        pack_scan_frag(scan_batch_hdr(1, 2, 1), std::slice::from_ref(&ap36)).expect("pack frag 1");
    let (hdr1, aps1) = unpack_scan_frag(&wire1).expect("unpack frag 1");
    let (_complete_hdr, complete_aps) =
        ingest_wifi_scan_frag(hdr1, aps1, &mut dedup, &mut reasm, 1_001).expect("reasm complete");
    assert_eq!(complete_aps.len(), 2);
    assert_eq!(complete_aps[0].channel, 6);
    assert_eq!(complete_aps[1].channel, 36);

    let hist = sample_gps_hist();
    let mut mac_table = MacTable::new(64, 16);
    let line6 = csv_line_for_ap(&complete_aps[0], &mut mac_table, &hist);
    let line36 = csv_line_for_ap(&complete_aps[1], &mut mac_table, &hist);
    assert_eq!(
        line6.trim(),
        include_str!("fixtures/wigle/wifi_row_minimal_pipeline.line").trim()
    );
    assert_eq!(
        line36.trim(),
        include_str!("fixtures/wigle/wifi_row_5ghz_ch36.line").trim()
    );
}

#[test]
fn wifi_channel_zero_maps_to_zero_frequency() {
    assert_eq!(wardriver_core::wigle::channel_to_frequency_mhz(0), 0);
    assert_eq!(wardriver_core::wigle::channel_to_frequency_mhz(255), 0);
}

#[test]
fn rcoi_tlv_survives_pack_reasm_and_csv() {
    let mpdu = include_bytes!("fixtures/ieee80211/beacon_rcoi_openroaming.bin");
    let mut ap = try_ap_from_mgmt_mpdu(mpdu, -43, 161).expect("beacon parse");
    ap.obs_mono_ms = OBS_MONO_MS;
    let extras = ScanEntryExtras {
        rcoi_ie_payload: roaming_consortium_ie_payload_from_mgmt_mpdu(mpdu).unwrap_or_default(),
        ble_cod: None,
    };
    let hdr = scan_batch_hdr(0, 1, 1);
    let wire = pack_scan_frag_with_extras(
        hdr,
        std::slice::from_ref(&ap),
        std::slice::from_ref(&extras),
    )
    .expect("pack tlv");
    let (back_hdr, aps, back_ex) = unpack_scan_frag_with_extras(&wire).expect("unpack tlv");
    let mut dedup = IndexMap::new();
    let mut reasm = HashMap::new();
    let (_complete_hdr, complete_aps, complete_ex) =
        ingest_wifi_scan_frag_with_extras(back_hdr, aps, back_ex, &mut dedup, &mut reasm, 1_000)
            .expect("reasm");
    assert_eq!(complete_aps.len(), 1);
    let rcois = rcois_from_ie_payload(&complete_ex[0].rcoi_ie_payload);
    assert_eq!(rcois, "5A03BA0000 BAA2D00000 BAA2D02000");
    let mut inp = ap_entry_to_wifi_line(
        &complete_aps[0],
        "2026-04-26 08:00:00",
        Some(37.7749),
        Some(-122.4194),
        Some(12),
        Some(7.5),
    );
    inp.rcois = rcois;
    let line = format_wifi_csv_line(&inp);
    assert!(line.contains("5A03BA0000 BAA2D00000 BAA2D02000"));
}
