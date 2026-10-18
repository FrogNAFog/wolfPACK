//! Host tests for `wardriver_core::lpr_camera_ble` method classification and cooldown.

use std::collections::HashSet;

use wardriver_core::detection::{
    ble_mask_needs_detection, detection_ble_disable_bit, DetectionBleMethod,
};
use wardriver_core::lpr_camera_ble::{
    LprCameraBleDetector, AXON_OUI, IPRO_OUI, PER_SRC_COOLDOWN_MS,
};
use wardriver_proto::BleEntryV1;

fn ble_entry(addr: [u8; 6], company_id: u16, name: &str) -> BleEntryV1 {
    let mut name_buf = [0u8; 22];
    let bytes = name.as_bytes();
    let len = bytes.len().min(22);
    name_buf[..len].copy_from_slice(&bytes[..len]);
    BleEntryV1 {
        addr,
        rssi: -55,
        addr_type: 0,
        company_id,
        name: name_buf,
        obs_mono_ms: 0,
    }
}

const ALL_BLE_METHODS_ENABLED: u32 = 0;

#[test]
fn xuntong_company_id_is_method_32() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0x09C8, "");
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("xuntong company id");
    assert_eq!(
        alert.method,
        DetectionBleMethod::XuntongManufacturerId.as_u8()
    );
}

#[test]
fn lpr_camera_keyword_in_name_is_method_33() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF], 0, "FLOCK-cam");
    let alert = det
        .ingest(2, 2_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("LPR camera keyword");
    assert_eq!(
        alert.method,
        DetectionBleMethod::AdvertisedNameKeyword.as_u8()
    );
}

#[test]
fn lpr_camera_infrastructure_oui_is_method_34() {
    let mut det = LprCameraBleDetector::new();
    // Prefix `70:C9:4E` from `lpr_camera_wifi` infrastructure allowlist (method 34).
    let entry = ble_entry([0x70, 0xC9, 0x4E, 0x01, 0x02, 0x03], 0, "");
    let alert = det
        .ingest(3, 3_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("LPR camera infra OUI");
    assert_eq!(
        alert.method,
        DetectionBleMethod::MacOuiInfrastructure.as_u8()
    );
}

#[test]
fn axon_oui_is_method_35() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC], 0, "");
    let alert = det
        .ingest(4, 4_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("axon oui");
    assert_eq!(alert.method, DetectionBleMethod::AxonOui.as_u8());
}

#[test]
fn ipro_oui_is_method_36() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0x01, 0x02, 0x03], 0, "");
    let alert = det
        .ingest(5, 5_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("ipro oui");
    assert_eq!(alert.method, DetectionBleMethod::IproOui.as_u8());
}

#[test]
fn per_src_cooldown_suppresses_repeat_alerts_for_30s() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0, "");
    let ignore = HashSet::new();

    let t0 = 10_000u64;
    assert!(det
        .ingest(1, t0, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
        .is_some());

    let inside = t0 + PER_SRC_COOLDOWN_MS - 1;
    assert!(det
        .ingest(1, inside, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
        .is_none());

    let at_boundary = t0 + PER_SRC_COOLDOWN_MS;
    assert!(det
        .ingest(1, at_boundary, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
        .is_some());
}

// ── Edge case tests ──

const METHOD_32: u8 = DetectionBleMethod::XuntongManufacturerId.as_u8();
const METHOD_35: u8 = DetectionBleMethod::AxonOui.as_u8();
const METHOD_36: u8 = DetectionBleMethod::IproOui.as_u8();

#[test]
fn all_ble_methods_disabled() {
    let mask = 0xFFFF_FFFFu32; // all 32 BLE method bits set
    assert!(!ble_mask_needs_detection(mask));
}

#[test]
fn axon_method_disabled_by_mask() {
    let mask = detection_ble_disable_bit(DetectionBleMethod::AxonOui.as_u8());
    let entry = ble_entry([0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), mask)
            .is_none(),
        "axon should be disabled by its own bit"
    );
}

#[test]
fn axon_oui_method_35_ignores_company_id() {
    let entry = ble_entry([0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC], 0x1234, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("axon oui should match regardless of company_id");
    assert_eq!(alert.method, DetectionBleMethod::AxonOui.as_u8());
}

#[test]
fn axon_oui_constant() {
    assert_eq!(AXON_OUI, [0x00, 0x25, 0xDF]);
}

#[test]
fn empty_name_no_keyword_match() {
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
            .is_none(),
        "empty name should not match LPR camera keywords"
    );
}

#[test]
fn lpr_camera_keyword_case_insensitive() {
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0, "fLoCk-test");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("case-insensitive LPR camera keyword");
    assert_eq!(
        alert.method,
        DetectionBleMethod::AdvertisedNameKeyword.as_u8()
    );
}

#[test]
fn ignore_set_suppresses_all_methods() {
    let ignore = HashSet::from([
        [0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC],
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
    ]);
    for addr in [
        [0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC],
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
    ] {
        let entry = ble_entry(addr, 0, "FLOCK");
        let mut det = LprCameraBleDetector::new();
        assert!(
            det.ingest(1, 1_000, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
                .is_none(),
            "ignored MAC should not trigger alerts"
        );
    }
}

#[test]
fn ipro_oui_method_36_ignores_company_id() {
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0xAA, 0xBB, 0xCC], 0x5678, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("ipro oui should match regardless of company_id");
    assert_eq!(alert.method, DetectionBleMethod::IproOui.as_u8());
}

#[test]
fn ipro_oui_constant() {
    assert_eq!(IPRO_OUI, [0xD4, 0x2D, 0xC5]);
}

#[test]
fn no_match_conditions() {
    let entry = ble_entry(
        [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
        0x1234,
        "Just-a-device",
    );
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
            .is_none(),
        "random MAC, name, company_id should not trigger"
    );
}

#[test]
fn partial_axon_oui_no_match() {
    let entry = ble_entry([0x00, 0x25, 0x00, 0xAA, 0xBB, 0xCC], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
            .is_none(),
        "partial axon OUI should not match"
    );
}

#[test]
fn partial_ipro_oui_no_match() {
    let entry = ble_entry([0xD4, 0x00, 0x00, 0xAA, 0xBB, 0xCC], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
            .is_none(),
        "partial ipro OUI should not match"
    );
}

#[test]
fn penguin_keyword_match() {
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0, "penguin-cam");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("penguin keyword");
    assert_eq!(
        alert.method,
        DetectionBleMethod::AdvertisedNameKeyword.as_u8()
    );
}

#[test]
fn per_src_cooldown_ms_constant() {
    assert_eq!(PER_SRC_COOLDOWN_MS, 30_000);
}

#[test]
fn pigvision_keyword_match() {
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0, "pigvision");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("pigvision keyword");
    assert_eq!(
        alert.method,
        DetectionBleMethod::AdvertisedNameKeyword.as_u8()
    );
}

#[test]
fn prune_removes_idle_entries() {
    let mut det = LprCameraBleDetector::new();
    let entry = ble_entry([0x11, 0x22, 0x33, 0x44, 0x55, 0x66], 0x09C8, "");
    let ignore = HashSet::new();
    // First ingest creates the entry and fires an alert
    assert!(det
        .ingest(1, 0, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
        .is_some());

    // Prune removes idle entries after 60s; after pruning, re-ingest should
    // recreate the cooldown state, so a second alert fires at 70s
    det.prune(70_000);
    assert!(det
        .ingest(1, 70_000, &entry, &ignore, ALL_BLE_METHODS_ENABLED)
        .is_some());
}

#[test]
fn xuntong_company_id_beats_ipro_oui() {
    // iPRO OUI with Xuntong company ID should give method 32, not 36
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0x01, 0x02, 0x03], 0x09C8, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), ALL_BLE_METHODS_ENABLED)
        .expect("xuntong company_id beats iPRO OUI");
    assert_eq!(
        alert.method,
        DetectionBleMethod::XuntongManufacturerId.as_u8()
    );
}

#[test]
fn xuntong_disabled_but_axon_oui_fires() {
    // With xuntong disabled, axon OUI should still fire as method 35
    let mask = detection_ble_disable_bit(DetectionBleMethod::XuntongManufacturerId.as_u8());
    let entry = ble_entry([0x00, 0x25, 0xDF, 0xAA, 0xBB, 0xCC], 0x09C8, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), mask)
        .expect("axon should fire when xuntong disabled");
    assert_eq!(alert.method, DetectionBleMethod::AxonOui.as_u8());
}

#[test]
fn method_constants() {
    assert_eq!(METHOD_32, 32);
    assert_eq!(METHOD_35, 35);
    assert_eq!(METHOD_36, 36);
}

#[test]
fn cooldown_constant() {
    assert_eq!(wardriver_core::lpr_camera_ble::PER_SRC_COOLDOWN_MS, 30_000);
}

#[test]
fn ble_mask_zero_enables_all() {
    assert!(ble_mask_needs_detection(0));
}

#[test]
fn ble_mask_all_bits_disables_all() {
    let mask = 0xFFFF_FFFFu32;
    assert!(!ble_mask_needs_detection(mask));
}

#[test]
fn default_ble_mask_allows_axon_and_ipro() {
    // Default disables 32, 33, 34 but leaves 35 (Axon) and 36 (i-PRO) enabled
    use wardriver_core::detection::DETECTION_BLE_DISABLE_MASK_DEFAULT;
    let mask = DETECTION_BLE_DISABLE_MASK_DEFAULT;
    assert_eq!(
        mask,
        detection_ble_disable_bit(32)
            | detection_ble_disable_bit(33)
            | detection_ble_disable_bit(34)
    );
    // Axon (35) and i-PRO (36) should be enabled by default
    assert!(!detection_ble_disable_bit(35) & mask != 0);
    assert!(!detection_ble_disable_bit(36) & mask != 0);
}

#[test]
fn ble_disable_bit_for_axon_and_ipro() {
    let axon_bit = detection_ble_disable_bit(METHOD_35);
    let ipro_bit = detection_ble_disable_bit(METHOD_36);
    assert_ne!(axon_bit, ipro_bit);
    assert_eq!(axon_bit, 1u32 << (35 - 32));
    assert_eq!(ipro_bit, 1u32 << (36 - 32));
}

#[test]
fn no_ble_methods_disabled_mask_zero() {
    let mask = 0u32;
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det.ingest(1, 1_000, &entry, &HashSet::new(), mask);
    assert!(alert.is_some(), "axon OUI should fire with mask=0");
    assert_eq!(alert.unwrap().method, METHOD_35);
}

#[test]
fn all_ble_methods_disabled_mask() {
    let mask = 0xFFFF_FFFFu32;
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), mask)
            .is_none(),
        "all methods disabled should suppress axon"
    );
}

#[test]
fn axon_method35_disabled_by_mask() {
    let mask = detection_ble_disable_bit(METHOD_35);
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), mask)
            .is_none(),
        "axon method 35 disabled by its bit"
    );
}

#[test]
fn ipro_method36_disabled_by_mask() {
    let mask = detection_ble_disable_bit(METHOD_36);
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0x01, 0x02, 0x03], 0, "");
    let mut det = LprCameraBleDetector::new();
    assert!(
        det.ingest(1, 1_000, &entry, &HashSet::new(), mask)
            .is_none(),
        "ipro method 36 disabled by its bit"
    );
}

#[test]
fn axon_oui_with_other_company_id_gets_method35() {
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0x1234, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("axon OUI with non-xuntong company_id should give method 35");
    assert_eq!(alert.method, METHOD_35);
}

#[test]
fn axon_oui_with_xuntong_company_id_gets_method32() {
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0x09C8, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("axon OUI with xuntong company_id should give method 32");
    assert_eq!(alert.method, METHOD_32);
}

#[test]
fn axon_oui_with_lpr_camera_name_no_xuntong_company_id() {
    // Name keyword (33) has higher priority than Axon OUI (35)
    let entry = ble_entry([0x00, 0x25, 0xDF, 0x01, 0x02, 0x03], 0x5678, "FLOCK");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("axon OUI with LPR camera name, non-xuntong CID");
    assert_eq!(
        alert.method,
        DetectionBleMethod::AdvertisedNameKeyword.as_u8()
    );
}

#[test]
fn ipro_oui_with_xuntong_company_id_gets_method32() {
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0x01, 0x02, 0x03], 0x09C8, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("ipro OUI with xuntong CID gets method 32");
    assert_eq!(alert.method, METHOD_32);
}

#[test]
fn ipro_oui_with_other_company_id_gets_method36() {
    let entry = ble_entry([0xD4, 0x2D, 0xC5, 0x01, 0x02, 0x03], 0x1234, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("ipro OUI with non-xuntong CID gets method 36");
    assert_eq!(alert.method, METHOD_36);
}

#[test]
fn non_surveillance_oui_with_xuntong_company_id_gets_method32() {
    let entry = ble_entry([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF], 0x09C8, "");
    let mut det = LprCameraBleDetector::new();
    let alert = det
        .ingest(1, 1_000, &entry, &HashSet::new(), 0)
        .expect("random OUI with xuntong CID gets method 32");
    assert_eq!(alert.method, METHOD_32);
}
