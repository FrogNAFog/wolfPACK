//! End-to-end host pipeline: BLE observation → ESP-NOW BleScanFrag → CSV line.
//!
//! NimBLE capture and SD FAT write are firmware-only; this stitches pack/unpack,
//! reassembly, GPS anchor, mac_table, and WiGLE BLE row encoding.

use std::collections::{HashMap, VecDeque};

use indexmap::IndexMap;
use wardriver_core::gps_anchor::{self, GpsFix, MAX_FIX_TIMELINE_DELTA_MS};
use wardriver_core::mac_table::{LaneFlags, MacTable};
use wardriver_core::scan_reasm::ingest_ble_scan_frag;
use wardriver_core::wigle::{ble_entry_to_ble_line, format_ble_csv_line};
use wardriver_core::wigle_ble::ble_csv_fields_from_cod;
use wardriver_proto::{
    pack_ble_scan_frag, pack_ble_scan_frag_with_extras, unpack_ble_scan_frag,
    unpack_ble_scan_frag_with_extras, BleEntryV1, BleScanFragV1Header, ScanEntryExtras,
    FRAG_FLAG_TRANSPORT_END,
};

const SCAN_END_UNIX_MS: u64 = 1_777_188_600_000; // 2026-04-26 07:30:00 UTC
const SCAN_END_MS: u32 = 30_000;
const OBS_MONO_MS: u32 = SCAN_END_MS;

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
    let mut hist = VecDeque::new();
    hist.push_back((
        SCAN_END_UNIX_MS as i64,
        GpsFix {
            utc_unix_ms: SCAN_END_UNIX_MS as i64,
            lat: 37.0,
            lon: -122.0,
            altitude_m: Some(10.0),
            hdop: Some(1.0),
            accuracy_m: None,
            fix_quality: 1,
        },
    ));
    hist
}

#[test]
fn pipeline_ble_to_csv_line() {
    let entry = sample_ble_entry();

    // Pack/unpack BLE scan fragment (sniffer → ESP-NOW wire).
    let hdr = BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 5,
        seq: 7,
        scan_batch_id: 9,
        sniffer_boot_id: 0x1122_3344,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    let wire = pack_ble_scan_frag(hdr, std::slice::from_ref(&entry)).expect("pack ble frag");
    let (back_hdr, entries) = unpack_ble_scan_frag(&wire).expect("unpack ble frag");
    assert_eq!(entries.len(), 1);

    // Control reassembly.
    let mut dedup = IndexMap::new();
    let mut reasm_ble = HashMap::new();
    let (complete_hdr, reasm) =
        ingest_ble_scan_frag(back_hdr, entries, &mut dedup, &mut reasm_ble, 2_000)
            .expect("ble reasm complete");
    assert_eq!(reasm.entries.len(), 1);
    let e = &reasm.entries[0];

    let obs_unix = gps_anchor::observed_unix_ms_from_anchor(
        complete_hdr.sniffer_scan_end_ms,
        complete_hdr.sniffer_scan_end_unix_ms,
        e.obs_mono_ms,
    )
    .expect("obs unix");
    let hist = sample_gps_hist();
    let fix = gps_anchor::match_gps_fix_for_obs(&hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS)
        .expect("gps match");

    let mut mac_table = MacTable::new(64, 16);
    let allow = mac_table.allow(
        &e.addr,
        fix.lat,
        fix.lon,
        LaneFlags::WIGLE_BLE,
        50.0,
        Some(e.rssi),
    );
    assert!(allow.decision.allow_row);

    let ts = gps_anchor::utc_string_from_unix_ms(obs_unix).expect("utc string");
    let line_inp = ble_entry_to_ble_line(
        e,
        &ts,
        Some(fix.lat),
        Some(fix.lon),
        fix.altitude_m.map(|a| a.round() as i32),
        fix.hdop.map(gps_anchor::hdop_to_accuracy_m),
    );
    let line = format_ble_csv_line(&line_inp);
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/ble_row_minimal.line").trim()
    );
}

#[test]
fn ble_cod_tlv_survives_pack_and_csv() {
    let entry = sample_ble_entry();
    let extras = ScanEntryExtras {
        rcoi_ie_payload: Vec::new(),
        ble_cod: Some([0x18, 0x04, 0x00]),
    };
    let hdr = BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id: 3,
        seq: 10,
        scan_batch_id: 20,
        sniffer_boot_id: 0xAABB_CCDD,
        sniffer_scan_end_ms: SCAN_END_MS,
        sniffer_scan_end_unix_ms: SCAN_END_UNIX_MS,
        frag_idx: 0,
        frag_total: 1,
        entry_count: 1,
    };
    let wire = pack_ble_scan_frag_with_extras(
        hdr,
        std::slice::from_ref(&entry),
        std::slice::from_ref(&extras),
    )
    .expect("pack ble tlv");
    let (_back_hdr, entries, back_ex) = unpack_ble_scan_frag_with_extras(&wire).expect("unpack");
    assert_eq!(back_ex[0].ble_cod, Some([0x18, 0x04, 0x00]));
    let (capabilities, frequency_code) = ble_csv_fields_from_cod(0x18, 0x04, 0x00);
    let mut inp = ble_entry_to_ble_line(
        &entries[0],
        "2026-04-26 08:15:00",
        Some(37.0),
        Some(-122.0),
        Some(10),
        Some(5.0),
    );
    inp.capabilities = capabilities;
    inp.frequency_code = Some(frequency_code);
    inp.name = "BT Headset".to_string();
    inp.addr = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
    inp.mfgr_id = String::new();
    inp.rssi = -55;
    let line = format_ble_csv_line(&inp);
    assert_eq!(
        line.trim(),
        include_str!("fixtures/wigle/ble_row_headphones.line").trim()
    );
}
