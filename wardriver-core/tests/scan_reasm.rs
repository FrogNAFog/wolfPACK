//! Host tests for ESP-NOW scan fragment dedup and reassembly (`wardriver_core::scan_reasm`).
//!
//! Mirrors control [`wardriver-control/src/control/espnow_rx.rs`] without stats or eviction.
//! Pipeline tests only send single-fragment batches; these cover multi-frag and error paths.

use std::collections::HashMap;

use indexmap::IndexMap;
use wardriver_core::scan_reasm::{
    ingest_ble_scan_frag, ingest_ble_scan_frag_with_extras, ingest_wifi_scan_frag,
    ingest_wifi_scan_frag_with_extras, ReasmBle, ReasmWifi,
};
use wardriver_proto::{
    ApEntryV1, AuthMini, BleEntryV1, BleScanFragV1Header, ScanFragV1Header, FRAG_FLAG_TRANSPORT_END,
};

const END_UNIX_MS: u64 = 1_777_190_400_000;
const END_MONO_MS: u32 = 30_000;
const BOOT_ID: u32 = 0xAABB_CCDD;
const NOW_MS: u64 = 5_000;

/// Build a WiFi scan fragment header with shared defaults.
fn wifi_hdr(
    sniffer_id: u32,
    scan_batch_id: u32,
    seq: u32,
    frag_idx: u8,
    frag_total: u8,
    ap_count: u8,
    boot_id: u32,
    end_unix_ms: u64,
) -> ScanFragV1Header {
    ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id: boot_id,
        sniffer_scan_end_ms: END_MONO_MS,
        sniffer_scan_end_unix_ms: end_unix_ms,
        frag_idx,
        frag_total,
        ap_count,
    }
}

/// Build a BLE scan fragment header with shared defaults.
fn ble_hdr(
    sniffer_id: u32,
    scan_batch_id: u32,
    seq: u32,
    frag_idx: u8,
    frag_total: u8,
    boot_id: u32,
    end_unix_ms: u64,
) -> BleScanFragV1Header {
    BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id: boot_id,
        sniffer_scan_end_ms: END_MONO_MS,
        sniffer_scan_end_unix_ms: end_unix_ms,
        frag_idx,
        frag_total,
        entry_count: 1,
    }
}

/// One WiFi AP per fragment; BSSID first byte identifies the fragment.
fn wifi_ap(frag_byte: u8) -> ApEntryV1 {
    ApEntryV1 {
        bssid: [frag_byte, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        channel: 6,
        rssi: -50,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid: {
            let mut s = [0u8; 32];
            s[..4].copy_from_slice(b"Test");
            s
        },
        obs_mono_ms: END_MONO_MS,
    }
}

/// 2.4 GHz AP row (matches ieee80211 ch6 fixture BSSID/SSID).
fn wifi_ap_2ghz(bssid_last: u8, channel: u8, ssid: &str, rssi: i8) -> ApEntryV1 {
    let sl = ssid.len().min(32);
    let mut ssid_buf = [0u8; 32];
    ssid_buf[..sl].copy_from_slice(&ssid.as_bytes()[..sl]);
    ApEntryV1 {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, bssid_last],
        channel,
        rssi,
        auth: AuthMini::Wpa2,
        ssid_len: sl as u8,
        ssid: ssid_buf,
        obs_mono_ms: END_MONO_MS,
    }
}

/// 5 GHz AP row for multi-AP reassembly tests (channel + BSSID suffix identify the row).
fn wifi_ap_5ghz(bssid_last: u8, channel: u8, ssid: &str, rssi: i8) -> ApEntryV1 {
    let sl = ssid.len().min(32);
    let mut ssid_buf = [0u8; 32];
    ssid_buf[..sl].copy_from_slice(&ssid.as_bytes()[..sl]);
    ApEntryV1 {
        bssid: [0xaa, 0xbb, 0xcc, 0xdd, 0xee, bssid_last],
        channel,
        rssi,
        auth: AuthMini::Wpa2,
        ssid_len: sl as u8,
        ssid: ssid_buf,
        obs_mono_ms: END_MONO_MS,
    }
}

/// One BLE observation per fragment; addr first byte identifies the fragment.
fn ble_entry(frag_byte: u8) -> BleEntryV1 {
    BleEntryV1 {
        addr: [frag_byte, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name: [0u8; 22],
        obs_mono_ms: END_MONO_MS,
    }
}

fn fresh_wifi_state() -> (
    IndexMap<wardriver_core::scan_reasm::DedupKey, u64>,
    HashMap<wardriver_core::scan_reasm::ReasmKey, ReasmWifi>,
) {
    (IndexMap::new(), HashMap::new())
}

fn fresh_ble_state() -> (
    IndexMap<wardriver_core::scan_reasm::DedupKey, u64>,
    HashMap<wardriver_core::scan_reasm::ReasmKey, ReasmBle>,
) {
    (IndexMap::new(), HashMap::new())
}

#[test]
fn reject_invalid_frag_index_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let hdr_zero = wifi_hdr(1, 10, 1, 0, 0, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(hdr_zero, vec![wifi_ap(1)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    assert!(reasm.is_empty());

    let hdr_oob = wifi_hdr(1, 10, 1, 2, 2, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(hdr_oob, vec![wifi_ap(1)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    assert!(reasm.is_empty());
}

#[test]
fn reject_invalid_frag_index_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let hdr_zero = ble_hdr(1, 10, 1, 0, 0, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(hdr_zero, vec![ble_entry(1)], &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert!(reasm.is_empty());

    let hdr_oob = ble_hdr(1, 10, 1, 3, 2, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(hdr_oob, vec![ble_entry(1)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    assert!(reasm.is_empty());
}

#[test]
fn single_frag_completes_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let hdr = wifi_hdr(2, 20, 5, 0, 1, 1, BOOT_ID, END_UNIX_MS);
    let result = ingest_wifi_scan_frag(hdr, vec![wifi_ap(0xAA)], &mut dedup, &mut reasm, NOW_MS);
    let (_complete_hdr, aps) = result.expect("single frag completes");
    assert_eq!(aps.len(), 1);
    assert_eq!(aps[0].bssid[0], 0xAA);
    assert!(reasm.is_empty(), "completed batch removes reasm entry");
}

#[test]
fn single_frag_completes_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let hdr = ble_hdr(2, 20, 5, 0, 1, BOOT_ID, END_UNIX_MS);
    let result = ingest_ble_scan_frag(hdr, vec![ble_entry(0x11)], &mut dedup, &mut reasm, NOW_MS);
    let (_complete_hdr, batch) = result.expect("single frag completes");
    assert_eq!(batch.entries.len(), 1);
    assert_eq!(batch.entries[0].addr[0], 0x11);
    assert!(reasm.is_empty());
}

#[test]
fn multi_frag_out_of_order_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let sid = 3u32;
    let batch = 30u32;
    let seq = 7u32;

    // Deliver fragments 2, 0, 1 — complete only after the last arrives.
    let h2 = wifi_hdr(sid, batch, seq, 2, 3, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(h2, vec![wifi_ap(0x02)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    let h0 = wifi_hdr(sid, batch, seq, 0, 3, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(h0, vec![wifi_ap(0x00)], &mut dedup, &mut reasm, NOW_MS + 1)
            .is_none()
    );
    let h1 = wifi_hdr(sid, batch, seq, 1, 3, 1, BOOT_ID, END_UNIX_MS);
    let (_hdr, aps) =
        ingest_wifi_scan_frag(h1, vec![wifi_ap(0x01)], &mut dedup, &mut reasm, NOW_MS + 2)
            .expect("third frag completes batch");

    assert_eq!(aps.len(), 3);
    let mut bssids: Vec<u8> = aps.iter().map(|ap| ap.bssid[0]).collect();
    bssids.sort();
    assert_eq!(bssids, vec![0x00, 0x01, 0x02]);
}

#[test]
fn multi_frag_out_of_order_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let sid = 3u32;
    let batch = 30u32;
    let seq = 7u32;

    let h2 = ble_hdr(sid, batch, seq, 2, 3, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(h2, vec![ble_entry(0x02)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    let h0 = ble_hdr(sid, batch, seq, 0, 3, BOOT_ID, END_UNIX_MS);
    assert!(ingest_ble_scan_frag(
        h0,
        vec![ble_entry(0x00)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    let h1 = ble_hdr(sid, batch, seq, 1, 3, BOOT_ID, END_UNIX_MS);
    let (_hdr, batch_out) = ingest_ble_scan_frag(
        h1,
        vec![ble_entry(0x01)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2,
    )
    .expect("third frag completes batch");

    assert_eq!(batch_out.entries.len(), 3);
    let mut addrs: Vec<u8> = batch_out.entries.iter().map(|e| e.addr[0]).collect();
    addrs.sort();
    assert_eq!(addrs, vec![0x00, 0x01, 0x02]);
}

#[test]
fn duplicate_frag_dedup_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let hdr = wifi_hdr(4, 40, 1, 0, 2, 1, BOOT_ID, END_UNIX_MS);

    assert!(
        ingest_wifi_scan_frag(hdr, vec![wifi_ap(0x10)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    // Replay same frag_idx — dedup rejects; AP list must not double.
    assert!(
        ingest_wifi_scan_frag(hdr, vec![wifi_ap(0x10)], &mut dedup, &mut reasm, NOW_MS + 1)
            .is_none()
    );

    let hdr1 = wifi_hdr(4, 40, 1, 1, 2, 1, BOOT_ID, END_UNIX_MS);
    let (_h, aps) = ingest_wifi_scan_frag(
        hdr1,
        vec![wifi_ap(0x11)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2,
    )
    .expect("second unique frag completes");
    assert_eq!(aps.len(), 2);
}

#[test]
fn duplicate_frag_dedup_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let hdr = ble_hdr(4, 40, 1, 0, 2, BOOT_ID, END_UNIX_MS);

    assert!(
        ingest_ble_scan_frag(hdr, vec![ble_entry(0x10)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );
    assert!(ingest_ble_scan_frag(
        hdr,
        vec![ble_entry(0x10)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());

    let hdr1 = ble_hdr(4, 40, 1, 1, 2, BOOT_ID, END_UNIX_MS);
    let (_h, batch) = ingest_ble_scan_frag(
        hdr1,
        vec![ble_entry(0x11)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2,
    )
    .expect("second unique frag completes");
    assert_eq!(batch.entries.len(), 2);
}

#[test]
fn inconsistent_frag_total_clears_state_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let h0 = wifi_hdr(5, 50, 1, 0, 2, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(h0, vec![wifi_ap(0x20)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    // Conflicting frag_total — state cleared; batch aborted.
    let h1_bad = wifi_hdr(5, 50, 1, 1, 3, 1, BOOT_ID, END_UNIX_MS);
    assert!(ingest_wifi_scan_frag(
        h1_bad,
        vec![wifi_ap(0x21)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty());

    // Fresh 3-frag batch on a new seq (dedup keys from the aborted batch are still occupied).
    let h0_new = wifi_hdr(5, 50, 2, 0, 3, 1, BOOT_ID, END_UNIX_MS);
    assert!(ingest_wifi_scan_frag(
        h0_new,
        vec![wifi_ap(0x30)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2
    )
    .is_none());
    let h1_new = wifi_hdr(5, 50, 2, 1, 3, 1, BOOT_ID, END_UNIX_MS);
    assert!(ingest_wifi_scan_frag(
        h1_new,
        vec![wifi_ap(0x31)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 3
    )
    .is_none());
    let h2_new = wifi_hdr(5, 50, 2, 2, 3, 1, BOOT_ID, END_UNIX_MS);
    let (_h, aps) = ingest_wifi_scan_frag(
        h2_new,
        vec![wifi_ap(0x32)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 4,
    )
    .expect("new batch completes");
    assert_eq!(aps.len(), 3);
}

#[test]
fn inconsistent_frag_total_clears_state_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let h0 = ble_hdr(5, 50, 1, 0, 2, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(h0, vec![ble_entry(0x20)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let h1_bad = ble_hdr(5, 50, 1, 1, 3, BOOT_ID, END_UNIX_MS);
    assert!(ingest_ble_scan_frag(
        h1_bad,
        vec![ble_entry(0x21)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty());

    let h0_new = ble_hdr(5, 50, 2, 0, 3, BOOT_ID, END_UNIX_MS);
    assert!(ingest_ble_scan_frag(
        h0_new,
        vec![ble_entry(0x30)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2
    )
    .is_none());
    let h1_new = ble_hdr(5, 50, 2, 1, 3, BOOT_ID, END_UNIX_MS);
    assert!(ingest_ble_scan_frag(
        h1_new,
        vec![ble_entry(0x31)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 3
    )
    .is_none());
    let h2_new = ble_hdr(5, 50, 2, 2, 3, BOOT_ID, END_UNIX_MS);
    let (_h, batch) = ingest_ble_scan_frag(
        h2_new,
        vec![ble_entry(0x32)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 4,
    )
    .expect("new batch completes");
    assert_eq!(batch.entries.len(), 3);
}

#[test]
fn inconsistent_scan_end_unix_clears_state_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let h0 = wifi_hdr(6, 60, 1, 0, 2, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(h0, vec![wifi_ap(0x40)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    // Wall-clock anchor changed mid-batch — inconsistent; state dropped.
    let h1_bad = wifi_hdr(6, 60, 1, 1, 2, 1, BOOT_ID, END_UNIX_MS + 1_000);
    assert!(ingest_wifi_scan_frag(
        h1_bad,
        vec![wifi_ap(0x41)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty());
}

#[test]
fn inconsistent_scan_end_unix_clears_state_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let h0 = ble_hdr(6, 60, 1, 0, 2, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(h0, vec![ble_entry(0x40)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let h1_bad = ble_hdr(6, 60, 1, 1, 2, BOOT_ID, END_UNIX_MS + 1_000);
    assert!(ingest_ble_scan_frag(
        h1_bad,
        vec![ble_entry(0x41)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty());
}

#[test]
fn boot_id_change_resets_batch_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let h0 = wifi_hdr(7, 70, 1, 0, 2, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(h0, vec![wifi_ap(0x50)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    // Sniffer rebooted — partial batch discarded; new boot_id starts fresh entry.
    let new_boot = BOOT_ID + 1;
    let h1_reset = wifi_hdr(7, 70, 1, 1, 2, 1, new_boot, END_UNIX_MS);
    assert!(ingest_wifi_scan_frag(
        h1_reset,
        vec![wifi_ap(0x51)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    // Only the rebooted frag is retained (frag 0 from old boot was lost).
    assert_eq!(reasm.len(), 1);
    let entry = reasm.values().next().expect("one in-flight batch");
    assert_eq!(entry.sniffer_boot_id, new_boot);
    assert_eq!(entry.got.len(), 1);
    assert_eq!(entry.aps.len(), 1);
    assert_eq!(entry.aps[0].bssid[0], 0x51);
}

#[test]
fn boot_id_change_resets_batch_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();
    let h0 = ble_hdr(7, 70, 1, 0, 2, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(h0, vec![ble_entry(0x50)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let new_boot = BOOT_ID + 1;
    let h1_reset = ble_hdr(7, 70, 1, 1, 2, new_boot, END_UNIX_MS);
    assert!(ingest_ble_scan_frag(
        h1_reset,
        vec![ble_entry(0x51)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert_eq!(reasm.len(), 1);
    let entry = reasm.values().next().expect("one in-flight batch");
    assert_eq!(entry.sniffer_boot_id, new_boot);
    assert_eq!(entry.got.len(), 1);
    assert_eq!(entry.entries.len(), 1);
    assert_eq!(entry.entries[0].addr[0], 0x51);
}

#[test]
fn parallel_batches_isolated_wifi() {
    let (mut dedup, mut reasm) = fresh_wifi_state();

    // Batch A: (batch=80, seq=1), two fragments.
    let a0 = wifi_hdr(8, 80, 1, 0, 2, 1, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_wifi_scan_frag(a0, vec![wifi_ap(0xA0)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    // Batch B: (batch=81, seq=2), single fragment — completes immediately.
    let b0 = wifi_hdr(8, 81, 2, 0, 1, 1, BOOT_ID, END_UNIX_MS);
    let (_hb, b_aps) =
        ingest_wifi_scan_frag(b0, vec![wifi_ap(0xB0)], &mut dedup, &mut reasm, NOW_MS + 1)
            .expect("batch B completes");
    assert_eq!(b_aps.len(), 1);
    assert_eq!(b_aps[0].bssid[0], 0xB0);

    // Finish batch A.
    let a1 = wifi_hdr(8, 80, 1, 1, 2, 1, BOOT_ID, END_UNIX_MS);
    let (_ha, a_aps) =
        ingest_wifi_scan_frag(a1, vec![wifi_ap(0xA1)], &mut dedup, &mut reasm, NOW_MS + 2)
            .expect("batch A completes");
    assert_eq!(a_aps.len(), 2);
    assert_eq!(a_aps[0].bssid[0], 0xA0);
    assert_eq!(a_aps[1].bssid[0], 0xA1);
}

#[test]
fn parallel_batches_isolated_ble() {
    let (mut dedup, mut reasm) = fresh_ble_state();

    let a0 = ble_hdr(8, 80, 1, 0, 2, BOOT_ID, END_UNIX_MS);
    assert!(
        ingest_ble_scan_frag(a0, vec![ble_entry(0xA0)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let b0 = ble_hdr(8, 81, 2, 0, 1, BOOT_ID, END_UNIX_MS);
    let (_hb, b_batch) = ingest_ble_scan_frag(
        b0,
        vec![ble_entry(0xB0)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1,
    )
    .expect("batch B completes");
    assert_eq!(b_batch.entries.len(), 1);
    assert_eq!(b_batch.entries[0].addr[0], 0xB0);

    let a1 = ble_hdr(8, 80, 1, 1, 2, BOOT_ID, END_UNIX_MS);
    let (_ha, a_batch) = ingest_ble_scan_frag(
        a1,
        vec![ble_entry(0xA1)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2,
    )
    .expect("batch A completes");
    assert_eq!(a_batch.entries.len(), 2);
    assert_eq!(a_batch.entries[0].addr[0], 0xA0);
    assert_eq!(a_batch.entries[1].addr[0], 0xA1);
}

#[test]
fn multi_ap_single_frag_5ghz_wifi() {
    // Sniffer may pack several 5 GHz observations into one ESP-NOW ScanFrag.
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let aps = vec![
        wifi_ap_5ghz(0x36, 36, "FiveG-Test", -62),
        wifi_ap_5ghz(0x95, 149, "FiveG-High", -58),
    ];
    let hdr = wifi_hdr(9, 90, 1, 0, 1, 2, BOOT_ID, END_UNIX_MS);
    let (_complete_hdr, merged) =
        ingest_wifi_scan_frag(hdr, aps, &mut dedup, &mut reasm, NOW_MS).expect("single frag");
    assert_eq!(merged.len(), 2);
    let channels: Vec<u8> = merged.iter().map(|ap| ap.channel).collect();
    assert_eq!(channels, vec![36, 149]);
    assert!(reasm.is_empty());
}

#[test]
fn multi_ap_multi_frag_5ghz_wifi() {
    // Large 5 GHz sweeps split APs across fragments; reassembly concatenates all rows.
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let sid = 9u32;
    let batch = 91u32;
    let seq = 3u32;

    let frag0_aps = vec![
        wifi_ap_5ghz(0x36, 36, "FiveG-Test", -62),
        wifi_ap_5ghz(0x37, 40, "FiveG-Mid", -65),
    ];
    let h0 = wifi_hdr(sid, batch, seq, 0, 2, 2, BOOT_ID, END_UNIX_MS);
    assert!(ingest_wifi_scan_frag(h0, frag0_aps, &mut dedup, &mut reasm, NOW_MS).is_none());

    let frag1_aps = vec![wifi_ap_5ghz(0x95, 149, "FiveG-High", -58)];
    let h1 = wifi_hdr(sid, batch, seq, 1, 2, 1, BOOT_ID, END_UNIX_MS);
    let (_hdr, merged) = ingest_wifi_scan_frag(h1, frag1_aps, &mut dedup, &mut reasm, NOW_MS + 1)
        .expect("second frag completes batch");

    assert_eq!(merged.len(), 3);
    let channels: Vec<u8> = merged.iter().map(|ap| ap.channel).collect();
    assert_eq!(channels, vec![36, 40, 149]);
    assert!(reasm.is_empty());
}

#[test]
fn multi_ap_single_frag_mixed_band_wifi() {
    // Dual-band sniffer: merge_wifi_sweep_groups can pack 2.4 + 5 GHz rows in one ScanFrag.
    let (mut dedup, mut reasm) = fresh_wifi_state();
    let aps = vec![
        wifi_ap_2ghz(0xff, 6, "Snoop", -45),
        wifi_ap_5ghz(0x36, 36, "FiveG-Test", -62),
    ];
    let hdr = wifi_hdr(10, 92, 1, 0, 1, 2, BOOT_ID, END_UNIX_MS);
    let (_complete_hdr, merged) =
        ingest_wifi_scan_frag(hdr, aps, &mut dedup, &mut reasm, NOW_MS).expect("single frag");
    assert_eq!(merged.len(), 2);
    let channels: Vec<u8> = merged.iter().map(|ap| ap.channel).collect();
    assert_eq!(channels, vec![6, 36]);
    assert!(reasm.is_empty());
}

use wardriver_proto::ScanEntryExtras;

fn edge_wifi_hdr(
    sniffer_id: u32,
    scan_batch_id: u32,
    seq: u32,
    frag_idx: u8,
    frag_total: u8,
    ap_count: u8,
) -> ScanFragV1Header {
    ScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id: BOOT_ID,
        sniffer_scan_end_ms: END_MONO_MS,
        sniffer_scan_end_unix_ms: END_UNIX_MS,
        frag_idx,
        frag_total,
        ap_count,
    }
}

fn edge_ble_hdr(
    sniffer_id: u32,
    scan_batch_id: u32,
    seq: u32,
    frag_idx: u8,
    frag_total: u8,
) -> BleScanFragV1Header {
    BleScanFragV1Header {
        flags: FRAG_FLAG_TRANSPORT_END,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id: BOOT_ID,
        sniffer_scan_end_ms: END_MONO_MS,
        sniffer_scan_end_unix_ms: END_UNIX_MS,
        frag_idx,
        frag_total,
        entry_count: 1,
    }
}

fn edge_wifi_ap(byte: u8) -> ApEntryV1 {
    ApEntryV1 {
        bssid: [byte, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        channel: 6,
        rssi: -50,
        auth: AuthMini::Wpa2,
        ssid_len: 4,
        ssid: {
            let mut s = [0u8; 32];
            s[..4].copy_from_slice(b"Test");
            s
        },
        obs_mono_ms: END_MONO_MS,
    }
}

fn edge_ble_entry(byte: u8) -> BleEntryV1 {
    BleEntryV1 {
        addr: [byte, 0x22, 0x33, 0x44, 0x55, 0x66],
        rssi: -70,
        addr_type: 1,
        company_id: 0x004C,
        name: [0u8; 22],
        obs_mono_ms: END_MONO_MS,
    }
}

fn edge_fresh_wifi() -> (
    IndexMap<(u32, u32, u32, u8), u64>,
    HashMap<(u32, u32, u32), ReasmWifi>,
) {
    (IndexMap::new(), HashMap::new())
}

fn edge_fresh_ble() -> (
    IndexMap<(u32, u32, u32, u8), u64>,
    HashMap<(u32, u32, u32), ReasmBle>,
) {
    (IndexMap::new(), HashMap::new())
}

// ── entry_count vs payload length ──────────────────────────────

#[test]
fn wifi_frag_header_ap_count_less_than_payload() {
    // Header claims 1 AP but we send 3 — ingest_wifi_scan_frag doesn't validate
    // ap_count against actual slice length; it just extends.
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(1, 10, 1, 0, 1, 1); // ap_count=1
    let aps = vec![edge_wifi_ap(1), edge_wifi_ap(2), edge_wifi_ap(3)];
    let (_h, merged) = ingest_wifi_scan_frag(hdr, aps, &mut dedup, &mut reasm, NOW_MS)
        .expect("single frag completes");
    // All 3 APs are kept; ap_count in header is informational
    assert_eq!(merged.len(), 3);
}

#[test]
fn wifi_frag_header_ap_count_more_than_payload() {
    // Header claims 5 APs but we send 2 — still accepted; ap_count is informational.
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(2, 20, 1, 0, 1, 5); // ap_count=5
    let aps = vec![edge_wifi_ap(1), edge_wifi_ap(2)];
    let (_h, merged) = ingest_wifi_scan_frag(hdr, aps, &mut dedup, &mut reasm, NOW_MS)
        .expect("single frag completes");
    assert_eq!(merged.len(), 2);
}

#[test]
fn ble_frag_entry_count_vs_payload_len_mismatch() {
    // BLE: entry_count in header vs actual entries slice length
    let (mut dedup, mut reasm) = edge_fresh_ble();
    let mut hdr = edge_ble_hdr(3, 30, 1, 0, 1);
    hdr.entry_count = 1;
    let entries = vec![edge_ble_entry(1), edge_ble_entry(2)];
    let (_h, batch) = ingest_ble_scan_frag(hdr, entries, &mut dedup, &mut reasm, NOW_MS)
        .expect("single frag completes");
    assert_eq!(batch.entries.len(), 2);
}

// ── empty fragment payloads ────────────────────────────────────

#[test]
fn wifi_frag_zero_aps_completes() {
    // Sniffer sends a scan with zero APs (empty channel dwell)
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(4, 40, 1, 0, 1, 0);
    let result = ingest_wifi_scan_frag(hdr, vec![], &mut dedup, &mut reasm, NOW_MS);
    let (_h, merged) = result.expect("zero-AP frag completes");
    assert!(merged.is_empty());
    assert!(reasm.is_empty());
}

#[test]
fn ble_frag_zero_entries_completes() {
    let (mut dedup, mut reasm) = edge_fresh_ble();
    let mut hdr = edge_ble_hdr(5, 50, 1, 0, 1);
    hdr.entry_count = 0;
    let result = ingest_ble_scan_frag(hdr, vec![], &mut dedup, &mut reasm, NOW_MS);
    let (_h, batch) = result.expect("zero-entry frag completes");
    assert!(batch.entries.is_empty());
    assert!(reasm.is_empty());
}

#[test]
fn wifi_multi_frag_with_empty_middle() {
    // 3-frag batch where frag 1 has zero APs
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let h0 = edge_wifi_hdr(6, 60, 1, 0, 3, 1);
    assert!(
        ingest_wifi_scan_frag(h0, vec![edge_wifi_ap(0)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let h1 = edge_wifi_hdr(6, 60, 1, 1, 3, 0); // empty
    assert!(ingest_wifi_scan_frag(h1, vec![], &mut dedup, &mut reasm, NOW_MS + 1).is_none());

    let h2 = edge_wifi_hdr(6, 60, 1, 2, 3, 1);
    let (_hdr, merged) = ingest_wifi_scan_frag(
        h2,
        vec![edge_wifi_ap(2)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2,
    )
    .expect("third frag completes");
    assert_eq!(merged.len(), 2); // frag 0's AP + frag 2's AP
}

// ── frag_idx at u8 boundaries ──────────────────────────────────

#[test]
fn wifi_frag_idx_254_accepted() {
    // frag_idx = 254, frag_total = 255 — valid (idx < total)
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(7, 70, 1, 254, 255, 1);
    assert!(
        ingest_wifi_scan_frag(hdr, vec![edge_wifi_ap(254)], &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert_eq!(reasm.len(), 1);
}

#[test]
fn wifi_frag_idx_255_rejected_when_frag_total_255() {
    // frag_idx = 255, frag_total = 255 — idx >= total → rejected
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(8, 80, 1, 255, 255, 1);
    assert!(
        ingest_wifi_scan_frag(hdr, vec![edge_wifi_ap(255)], &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert!(reasm.is_empty());
}

#[test]
fn ble_frag_idx_255_rejected_when_frag_total_255() {
    let (mut dedup, mut reasm) = edge_fresh_ble();
    let hdr = edge_ble_hdr(9, 90, 1, 255, 255);
    assert!(
        ingest_ble_scan_frag(hdr, vec![edge_ble_entry(1)], &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert!(reasm.is_empty());
}

// ── reassembly eviction / stale cleanup ────────────────────────

#[test]
fn reasm_map_grows_with_incomplete_batches() {
    // Multiple incomplete batches occupy reasm entries
    let (mut dedup, mut reasm) = edge_fresh_wifi();

    // Start 3 independent batches
    let h1 = edge_wifi_hdr(10, 100, 1, 0, 2, 1);
    assert!(
        ingest_wifi_scan_frag(h1, vec![edge_wifi_ap(1)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let h2 = edge_wifi_hdr(10, 200, 2, 0, 2, 1);
    assert!(ingest_wifi_scan_frag(
        h2,
        vec![edge_wifi_ap(2)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());

    let h3 = edge_wifi_hdr(10, 300, 3, 0, 2, 1);
    assert!(ingest_wifi_scan_frag(
        h3,
        vec![edge_wifi_ap(3)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 2
    )
    .is_none());

    assert_eq!(reasm.len(), 3);
}

#[test]
fn inconsistent_frag_total_removes_entry_from_reasm() {
    // After inconsistency, the reasm key is gone
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let h0 = edge_wifi_hdr(11, 110, 1, 0, 2, 1);
    assert!(
        ingest_wifi_scan_frag(h0, vec![edge_wifi_ap(0)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    // Second frag claims frag_total=3 instead of 2
    let h1_bad = edge_wifi_hdr(11, 110, 1, 1, 3, 1);
    assert!(ingest_wifi_scan_frag(
        h1_bad,
        vec![edge_wifi_ap(1)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty(), "inconsistent batch evicted");
}

#[test]
fn inconsistent_scan_end_unix_removes_entry() {
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let h0 = edge_wifi_hdr(12, 120, 1, 0, 2, 1);
    assert!(
        ingest_wifi_scan_frag(h0, vec![edge_wifi_ap(0)], &mut dedup, &mut reasm, NOW_MS).is_none()
    );

    let h1 = edge_wifi_hdr(12, 120, 1, 1, 2, 1);
    // We reuse edge_wifi_hdr which sets end_unix_ms from param — set it differently
    let mut h1_bad = edge_wifi_hdr(12, 120, 1, 1, 2, 1);
    h1_bad.sniffer_scan_end_unix_ms = END_UNIX_MS + 10_000;
    assert!(ingest_wifi_scan_frag(
        h1_bad,
        vec![edge_wifi_ap(1)],
        &mut dedup,
        &mut reasm,
        NOW_MS + 1
    )
    .is_none());
    assert!(reasm.is_empty(), "inconsistent scan end evicted");
}

// ── extras/extras length mismatch ──────────────────────────────

#[test]
fn wifi_frag_extras_length_mismatch_rejected() {
    let (mut dedup, mut reasm) = edge_fresh_wifi();
    let hdr = edge_wifi_hdr(13, 130, 1, 0, 1, 2);
    let aps = vec![edge_wifi_ap(1), edge_wifi_ap(2)];
    let extras = vec![ScanEntryExtras::default()]; // only 1 extra for 2 APs
    assert!(
        ingest_wifi_scan_frag_with_extras(hdr, aps, extras, &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert!(reasm.is_empty());
}

#[test]
fn ble_frag_extras_length_mismatch_rejected() {
    let (mut dedup, mut reasm) = edge_fresh_ble();
    let mut hdr = edge_ble_hdr(14, 140, 1, 0, 1);
    hdr.entry_count = 2;
    let entries = vec![edge_ble_entry(1), edge_ble_entry(2)];
    let extras = vec![ScanEntryExtras::default()]; // only 1
    assert!(
        ingest_ble_scan_frag_with_extras(hdr, entries, extras, &mut dedup, &mut reasm, NOW_MS)
            .is_none()
    );
    assert!(reasm.is_empty());
}
