//! Mobile WiFi ingest parity: PackLink bulk unpack → enrichment gates → WiGLE CSV.
//!
//! Mirrors control [`wardriver-core/tests/pipeline_wifi_5ghz_to_sd.rs`] semantics on the
//! phone path. Firmware stages (promisc RX, ESP-NOW TX, BLE notify) are out of scope.

use std::collections::VecDeque;

use wardriver_core::gps_anchor::{utc_string_from_unix_ms, GpsFix};
use wardriver_core::ieee80211_parse::try_ap_from_mgmt_mpdu;
use wardriver_core::wigle::{ap_entry_to_wifi_line, format_wifi_csv_line};
use wardriver_mobile_core::enrichment::{
    enrich_wifi_ap_with_rcois, EnrichOutcome, EnrichmentState,
};
use wardriver_mobile_core::observation::Observation;
use wardriver_packlink::{
    flags, pack_observation_bulk, unpack_frame, unpack_observation_bulk, MsgType, ObservationBulkV1,
};
use wardriver_proto::{
    pack_scan_frag, unpack_scan_frag, ApEntryV1, ScanFragV1Header, FRAG_FLAG_TRANSPORT_END,
};

/// Build a WiGLE WiFi CSV line for a stored observation (mirrors the removed
/// enrichment host-test converter).
fn wifi_csv_line(obs: &Observation, ap: &ApEntryV1) -> String {
    let ts = utc_string_from_unix_ms(obs.observed_unix_ms).expect("timestamp");
    let mut inp = ap_entry_to_wifi_line(
        ap,
        &ts,
        Some(obs.lat),
        Some(obs.lon),
        obs.altitude_m,
        obs.accuracy_m,
    );
    inp.rcois = obs.rcois.clone();
    format_wifi_csv_line(&inp)
}

// Same anchor as control pipeline goldens (2026-04-26 08:00:00 UTC).
const SCAN_END_UNIX_MS: u64 = 1_777_190_400_000;
const SCAN_END_MS: u32 = 50_000;
const OBS_MONO_MS: u32 = SCAN_END_MS;
const SNIFFER_ID: u32 = 3;

/// GPS timeline with one fix at the scan-end anchor — matches control `sample_gps_hist`.
fn sample_gps_hist() -> VecDeque<(i64, GpsFix)> {
    let mut hist = VecDeque::new();
    hist.push_back((
        SCAN_END_UNIX_MS as i64,
        GpsFix {
            utc_unix_ms: SCAN_END_UNIX_MS as i64,
            lat: 37.7749,
            lon: -122.4194,
            altitude_m: Some(12.0),
            hdop: None,
            accuracy_m: Some(7.5),
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

fn scan_batch_hdr(ap_count: u8) -> ScanFragV1Header {
    ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: SNIFFER_ID,
        seq: 10,
        scan_batch_id: 20,
        sniffer_boot_id: 0xAABB_CCDD,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx: 0,
        frag_total: 1,
        ap_count,
    }
}

/// Pack APs the way the gateway does after a complete single-frag batch.
fn pack_gateway_bulk(aps: &[ApEntryV1]) -> Vec<u8> {
    let hdr = scan_batch_hdr(aps.len() as u8);
    let scan_wire = pack_scan_frag(hdr, aps).expect("pack scan frag");
    let bulk = ObservationBulkV1 {
        batch_id: hdr.scan_batch_id,
        sniffer_id: hdr.sniffer_id,
        scan_end_unix_ms: hdr.sniffer_scan_end_unix_ms,
        wifi_ap_count: aps.len() as u16,
        ble_count: 0,
        payload: scan_wire,
    };
    pack_observation_bulk(&bulk, 1, flags::FRAGMENT_END)
}

#[test]
#[ignore = "manual: prints PackLink ObservationBulk wire hex for Android instrumented tests"]
fn print_gateway_bulk_wire_hex() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -55,
    );
    let wire = pack_gateway_bulk(&[ap]);
    for b in &wire {
        print!("{:02x}", b);
    }
    println!();
}

/// Phone path: unpack PackLink bulk → scan frag → enrich each AP → WiGLE CSV line.
fn mobile_csv_lines_from_bulk(
    bulk_wire: &[u8],
    state: &mut EnrichmentState,
    session_id: i64,
) -> Vec<String> {
    // Gate: PackLink frame must be ObservationBulk with valid CRC.
    let frame = unpack_frame(bulk_wire).expect("packlink frame");
    assert_eq!(frame.msg_type, MsgType::ObservationBulk);

    // Gate: bulk header + opaque scan payload.
    let bulk = unpack_observation_bulk(frame.payload).expect("bulk header");
    let (hdr, aps) = unpack_scan_frag(&bulk.payload).expect("scan frag in bulk");

    let mut lines = Vec::with_capacity(aps.len());
    for ap in &aps {
        // Gate chain inside the WiFi enrich: csv_enqueue, denylists, GPS anchor, MacTable dedup.
        match enrich_wifi_ap_with_rcois(
            state,
            session_id,
            hdr.sniffer_scan_end_ms,
            hdr.sniffer_scan_end_unix_ms,
            bulk.sniffer_id,
            ap,
            "",
        ) {
            EnrichOutcome::Stored(obs) => {
                let line = wifi_csv_line(&obs, ap);
                lines.push(line);
            }
            EnrichOutcome::StoredCell(_) => {
                panic!("unexpected cell outcome in wifi pipeline");
            }
            EnrichOutcome::StoredProbe(_) => {
                panic!("unexpected probe outcome in wifi pipeline");
            }
            EnrichOutcome::Dropped(reason) => {
                panic!("unexpected drop for {reason}");
            }
        }
    }
    lines
}

fn assert_line_matches_golden(line: &str, golden: &str) {
    assert_eq!(line.trim(), golden.trim());
}

#[test]
fn pipeline_mobile_5ghz_ch36_to_csv_line() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    assert_eq!(lines.len(), 1);
    assert_line_matches_golden(
        &lines[0],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_5ghz_ch36.line"),
    );
}

#[test]
fn pipeline_mobile_5ghz_ch149_to_csv_line() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch149.bin"),
        149,
        -58,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    assert_eq!(lines.len(), 1);
    assert_line_matches_golden(
        &lines[0],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_5ghz_ch149.line"),
    );
}

#[test]
fn pipeline_mobile_2ghz_ch6_to_csv_line() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    assert_eq!(lines.len(), 1);
    assert_line_matches_golden(
        &lines[0],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_minimal_pipeline.line"),
    );
}

#[test]
fn pipeline_mobile_two_aps_same_bulk_to_csv_lines() {
    // Two UNII APs in one single-frag batch — gateway exports one bulk with both APs.
    let ap36 = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let ap149 = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch149.bin"),
        149,
        -58,
    );
    let bulk_wire = pack_gateway_bulk(&[ap36, ap149]);
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    // Gate: each AP in bulk produces one observation row when MacTable allows.
    assert_eq!(lines.len(), 2);
    assert_eq!(state.next_id - 1, 2);
    assert_line_matches_golden(
        &lines[0],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_5ghz_ch36.line"),
    );
    assert_line_matches_golden(
        &lines[1],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_5ghz_ch149.line"),
    );
}

#[test]
fn pipeline_mobile_mixed_band_same_bulk_to_csv_lines() {
    let ap6 = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let ap36 = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_wpa2_ch36.bin"),
        36,
        -62,
    );
    let bulk_wire = pack_gateway_bulk(&[ap6, ap36]);
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    assert_eq!(lines.len(), 2);
    assert_line_matches_golden(
        &lines[0],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_minimal_pipeline.line"),
    );
    assert_line_matches_golden(
        &lines[1],
        include_str!("../../wardriver-core/tests/fixtures/wigle/wifi_row_5ghz_ch36.line"),
    );
}

#[test]
fn pipeline_mobile_mac_table_dedup_within_radius() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let lines = mobile_csv_lines_from_bulk(&bulk_wire, &mut state, 1);
    assert_eq!(lines.len(), 1);

    // ~30 m north — still within 50 m revisit; second bulk must not add a row.
    let mut near_hist = sample_gps_hist();
    near_hist[0].1.lat = 37.77515;
    state.gps_hist = near_hist;
    let frame = unpack_frame(&bulk_wire).expect("frame");
    let bulk = unpack_observation_bulk(frame.payload).expect("bulk");
    let (hdr, aps) = unpack_scan_frag(&bulk.payload).expect("scan");
    match enrich_wifi_ap_with_rcois(
        &mut state,
        1,
        hdr.sniffer_scan_end_ms,
        hdr.sniffer_scan_end_unix_ms,
        bulk.sniffer_id,
        &aps[0],
        "",
    ) {
        EnrichOutcome::Dropped("mac_table_dedup") => {}
        other => panic!("expected MacTable dedup within revisit radius, got {other:?}"),
    }
    assert_eq!(state.next_id - 1, 1);
}

#[test]
fn pipeline_mobile_csv_enqueue_blocked_in_geofence() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();
    // Gate: inside home geofence blocks csv enqueue before MacTable.
    state.inside_home_geofence = true;

    let frame = unpack_frame(&bulk_wire).expect("frame");
    let bulk = unpack_observation_bulk(frame.payload).expect("bulk");
    let (hdr, aps) = unpack_scan_frag(&bulk.payload).expect("scan");
    match enrich_wifi_ap_with_rcois(
        &mut state,
        1,
        hdr.sniffer_scan_end_ms,
        hdr.sniffer_scan_end_unix_ms,
        bulk.sniffer_id,
        &aps[0],
        "",
    ) {
        EnrichOutcome::Dropped("csv_blocked_home_zone") => {}
        other => panic!("expected geofence block, got {other:?}"),
    }
    assert_eq!(state.next_id, 1);
}

#[test]
fn pipeline_mobile_native_accuracy_below_two_point_five_meters() {
    let ap = ap_from_beacon(
        include_bytes!("../../wardriver-core/tests/fixtures/ieee80211/beacon_open_wpa2_ch6.bin"),
        6,
        -45,
    );
    let bulk_wire = pack_gateway_bulk(std::slice::from_ref(&ap));
    let mut state = EnrichmentState::default();
    state.gps_hist = VecDeque::from([(
        SCAN_END_UNIX_MS as i64,
        GpsFix {
            utc_unix_ms: SCAN_END_UNIX_MS as i64,
            lat: 37.7749,
            lon: -122.4194,
            altitude_m: Some(12.0),
            hdop: None,
            accuracy_m: Some(1.0),
            fix_quality: 1,
        },
    )]);

    let frame = unpack_frame(&bulk_wire).expect("frame");
    let bulk = unpack_observation_bulk(frame.payload).expect("bulk");
    let (hdr, aps) = unpack_scan_frag(&bulk.payload).expect("scan");
    let EnrichOutcome::Stored(obs) = enrich_wifi_ap_with_rcois(
        &mut state,
        1,
        hdr.sniffer_scan_end_ms,
        hdr.sniffer_scan_end_unix_ms,
        bulk.sniffer_id,
        &aps[0],
        "",
    ) else {
        panic!("expected stored observation");
    };
    assert!((obs.accuracy_m.unwrap() - 1.0).abs() < f32::EPSILON);

    let line = wifi_csv_line(&obs, &aps[0]);
    // AccuracyMeters column is the 11th field (0-indexed: 10).
    let acc_field = line.split(',').nth(10).expect("AccuracyMeters");
    assert_eq!(acc_field, "1");
}
