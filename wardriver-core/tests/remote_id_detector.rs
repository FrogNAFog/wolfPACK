mod remote_id_helpers;

use remote_id_helpers::{
    beacon_with_vendor_ie, ble_service_data_adv, sample_basic_id_msg, sample_location_msg,
    sample_message_pack,
};
use wardriver_core::detection::{
    remote_id_wifi_disable_bit, DetectionSignalKind, RemoteIdBleDetectionMethod,
    RemoteIdWifiDetectionMethod,
};
use wardriver_core::remote_id::{RemoteIdDetector, PER_SRC_COOLDOWN_MS};
use wardriver_proto::BleEntryV1;

#[test]
fn detector_merge_pack_and_cooldown() {
    let src = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01];
    let pack = sample_message_pack(&[
        sample_basic_id_msg(b"COOLDOWN-TEST"),
        sample_location_msg(10.0, 20.0, 15.0),
    ]);
    let adv = ble_service_data_adv(&pack);
    let entry = BleEntryV1 {
        addr: src,
        rssi: -55,
        addr_type: 0,
        company_id: 0,
        name: [0; 22],
        obs_mono_ms: 0,
    };
    let mut det = RemoteIdDetector::new();
    let a1 = det
        .ingest_ble(1, 1000, &entry, &adv, 0)
        .expect("first alert");
    assert_eq!(
        a1.method,
        RemoteIdBleDetectionMethod::ServiceDataFffa.as_u8()
    );
    assert_eq!(a1.signal_kind, DetectionSignalKind::Ble);
    assert!(det.ingest_ble(1, 2000, &entry, &adv, 0).is_none());
    let a2 = det
        .ingest_ble(1, 1000 + PER_SRC_COOLDOWN_MS, &entry, &adv, 0)
        .expect("after cooldown");
    assert_eq!(a2.src, src);
}

#[test]
fn detector_respects_disable_mask() {
    let src = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    let pack = sample_message_pack(&[
        sample_basic_id_msg(b"MASK-TEST"),
        sample_location_msg(1.0, 2.0, 3.0),
    ]);
    let mpdu = beacon_with_vendor_ie(src, &pack);
    let mut det = RemoteIdDetector::new();
    let mask = remote_id_wifi_disable_bit(RemoteIdWifiDetectionMethod::BeaconVendorIe.as_u8());
    assert!(det.ingest_wifi(2, 500, &mpdu, src, -60, 6, mask).is_none());
    assert!(det.ingest_wifi(2, 500, &mpdu, src, -60, 6, 0).is_some());
}

#[test]
fn detector_requires_location_and_id() {
    let src = [0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA];
    let basic_only = sample_basic_id_msg(b"NO-LOC");
    let adv = ble_service_data_adv(&basic_only);
    let entry = BleEntryV1 {
        addr: src,
        rssi: -40,
        addr_type: 0,
        company_id: 0,
        name: [0; 22],
        obs_mono_ms: 0,
    };
    let mut det = RemoteIdDetector::new();
    assert!(det.ingest_ble(1, 0, &entry, &adv, 0).is_none());
}

#[test]
fn detector_ble_legacy_method_id() {
    let src = [0x11, 0x11, 0x11, 0x11, 0x11, 0x11];
    let pack = sample_message_pack(&[
        sample_basic_id_msg(b"LEG"),
        sample_location_msg(0.0, 0.0, 0.0),
    ]);
    let mut adv = Vec::new();
    let inner = 2 + 1 + pack.len();
    adv.push((inner + 1) as u8);
    adv.push(0xFF);
    adv.extend_from_slice(&[0x02, 0x00, 0x0D]);
    adv.extend_from_slice(&pack);
    let entry = BleEntryV1 {
        addr: src,
        rssi: -70,
        addr_type: 0,
        company_id: 0,
        name: [0; 22],
        obs_mono_ms: 0,
    };
    let mut det = RemoteIdDetector::new();
    let a = det.ingest_ble(3, 0, &entry, &adv, 0).expect("legacy");
    assert_eq!(a.method, RemoteIdBleDetectionMethod::LegacyMfg0002.as_u8());
}
use wardriver_core::remote_id::extract_ble_remote_id_payload;
use wardriver_core::remote_id::{extract_wifi_remote_id_from_mgmt, RemoteIdWifiTransport};

use wardriver_core::remote_id::is_remote_id_wifi_channel;

#[test]
fn only_channels_6_and_149() {
    assert!(is_remote_id_wifi_channel(6));
    assert!(is_remote_id_wifi_channel(149));
    assert!(!is_remote_id_wifi_channel(1));
    assert!(!is_remote_id_wifi_channel(36));
    assert!(!is_remote_id_wifi_channel(0));
}

#[test]
fn extract_service_data_fffa() {
    let pack = sample_message_pack(&[
        sample_basic_id_msg(b"BLE-RID"),
        sample_location_msg(1.0, 2.0, 10.0),
    ]);
    let adv = ble_service_data_adv(&pack);
    let payload = extract_ble_remote_id_payload(&adv).expect("service data payload");
    assert_eq!(payload, &pack[..]);
}

#[test]
fn extract_legacy_mfg_0002() {
    let basic = sample_basic_id_msg(b"LEGACY");
    let mut adv = Vec::new();
    let inner = 2 + 1 + basic.len();
    adv.push((inner + 1) as u8);
    adv.push(0xFF);
    adv.extend_from_slice(&[0x02, 0x00, 0x0D]);
    adv.extend_from_slice(&basic);
    let payload = extract_ble_remote_id_payload(&adv).expect("legacy mfg");
    assert_eq!(payload, &basic[..]);
}

#[test]
fn extract_beacon_vendor_ie_ch6_fixture() {
    let src = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF];
    let pack = sample_message_pack(&[
        sample_basic_id_msg(b"WIFI-RID-CH6"),
        sample_location_msg(38.0, -90.0, 80.0),
    ]);
    let mpdu = beacon_with_vendor_ie(src, &pack);
    let (payload, transport) = extract_wifi_remote_id_from_mgmt(&mpdu).expect("beacon extract");
    assert_eq!(transport, RemoteIdWifiTransport::Beacon);
    assert_eq!(payload, &pack[..]);
}

#[test]
fn extract_beacon_vendor_ie_ch149_fixture() {
    let src = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let loc = sample_location_msg(33.9, -118.4, 30.0);
    let mpdu = beacon_with_vendor_ie(src, &loc);
    let (payload, transport) = extract_wifi_remote_id_from_mgmt(&mpdu).expect("ch149 beacon");
    assert_eq!(transport, RemoteIdWifiTransport::Beacon);
    assert_eq!(payload.len(), 25);
}
