//! Mobile BLE ingest parity: PackLink bulk unpack → enrichment gates → WiGLE CSV.
//!
//! Mirrors control [`wardriver-core/tests/pipeline_ble_to_sd.rs`] on the phone path.

use std::collections::VecDeque;

use wardriver_core::gps_anchor::{utc_string_from_unix_ms, GpsFix};
use wardriver_core::wigle::{ble_entry_to_ble_line, format_ble_csv_line};
use wardriver_mobile_core::enrichment::{
    enrich_ble_entry_with_extras, EnrichOutcome, EnrichmentState,
};
use wardriver_mobile_core::jni_enrich::unpack_observation_bulk_json;
use wardriver_mobile_core::observation::Observation;
use wardriver_packlink::{
    flags, pack_observation_bulk, unpack_frame, unpack_observation_bulk, MsgType, ObservationBulkV1,
};
use wardriver_proto::{
    pack_ble_scan_frag, unpack_ble_scan_frag, BleEntryV1, BleScanFragV1Header, ScanEntryExtras,
    FRAG_FLAG_TRANSPORT_END,
};

/// Build a WiGLE BLE CSV line for a stored observation (mirrors the removed
/// enrichment host-test converter).
fn ble_csv_line(obs: &Observation, entry: &BleEntryV1) -> String {
    let ts = utc_string_from_unix_ms(obs.observed_unix_ms).expect("timestamp");
    let mut inp = ble_entry_to_ble_line(
        entry,
        &ts,
        Some(obs.lat),
        Some(obs.lon),
        obs.altitude_m,
        obs.accuracy_m,
    );
    inp.capabilities = obs.ble_capabilities.clone();
    inp.frequency_code = if obs.frequency_mhz != 0 {
        Some(obs.frequency_mhz as u16)
    } else {
        Some(wardriver_core::wigle::BLE_DEFAULT_FREQUENCY_CODE)
    };
    format_ble_csv_line(&inp)
}

const SCAN_END_UNIX_MS: u64 = 1_777_188_600_000;
const SCAN_END_MS: u32 = 30_000;
const OBS_MONO_MS: u32 = SCAN_END_MS;
const SNIFFER_ID: u32 = 5;

fn sample_ble_entry() -> BleEntryV1 {
    let mut name = [0u8; 22];
    name[..10].copy_from_slice(b"Pixel Buds");
    BleEntryV1 {
        addr: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name,
        obs_mono_ms: OBS_MONO_MS,
    }
}

fn sample_gps_hist() -> VecDeque<(i64, GpsFix)> {
    VecDeque::from([(
        SCAN_END_UNIX_MS as i64,
        GpsFix {
            utc_unix_ms: SCAN_END_UNIX_MS as i64,
            lat: 37.0,
            lon: -122.0,
            altitude_m: Some(10.0),
            hdop: None,
            // Native fused accuracy (mobile path); matches control golden AccuracyMeters.
            accuracy_m: Some(5.0),
            fix_quality: 1,
        },
    )])
}

fn pack_gateway_ble_bulk(entry: &BleEntryV1) -> Vec<u8> {
    let hdr = BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: SNIFFER_ID,
        seq: 7,
        scan_batch_id: 9,
        sniffer_boot_id: 0x1122_3344,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    let scan_wire = pack_ble_scan_frag(hdr, std::slice::from_ref(entry)).expect("pack ble");
    let bulk = ObservationBulkV1 {
        batch_id: hdr.scan_batch_id,
        sniffer_id: hdr.sniffer_id,
        scan_end_unix_ms: hdr.sniffer_scan_end_unix_ms,
        wifi_ap_count: 0,
        ble_count: 1,
        payload: scan_wire,
    };
    pack_observation_bulk(&bulk, 1, flags::FRAGMENT_END)
}

#[test]
fn pipeline_mobile_ble_bulk_json_unpack_has_bles() {
    let entry = sample_ble_entry();
    let wire = pack_gateway_ble_bulk(&entry);
    let json = unpack_observation_bulk_json(&wire);
    assert!(json.contains("\"bles\""));
    assert!(json.contains("Pixel Buds"));
    assert!(!json.contains("\"scan_parse_error\""));
}

#[test]
fn pipeline_mobile_ble_to_csv_line() {
    let entry = sample_ble_entry();
    let bulk_wire = pack_gateway_ble_bulk(&entry);
    let mut state = EnrichmentState::default();
    state.gps_hist = sample_gps_hist();

    let frame = unpack_frame(&bulk_wire).expect("packlink frame");
    assert_eq!(frame.msg_type, MsgType::ObservationBulk);
    let bulk = unpack_observation_bulk(frame.payload).expect("bulk header");
    let (hdr, entries) = unpack_ble_scan_frag(&bulk.payload).expect("ble frag");
    assert_eq!(entries.len(), 1);

    let EnrichOutcome::Stored(obs) = enrich_ble_entry_with_extras(
        &mut state,
        1,
        hdr.sniffer_scan_end_ms,
        hdr.sniffer_scan_end_unix_ms,
        bulk.sniffer_id,
        &entries[0],
        &ScanEntryExtras::default(),
    ) else {
        panic!("expected stored BLE observation");
    };
    assert_eq!(obs.signal_kind.as_str(), "BLE");
    assert_eq!(obs.auth_mode, "76");

    let line = ble_csv_line(&obs, &entries[0]);
    assert_eq!(
        line.trim(),
        include_str!("../../wardriver-core/tests/fixtures/wigle/ble_row_minimal.line").trim()
    );
}
