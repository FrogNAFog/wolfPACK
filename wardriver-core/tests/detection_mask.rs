//! Host tests for detection method mask edge cases.

use wardriver_core::detection::{
    ble_mask_needs_detection, ble_method_caps, detection_ble_disable_bit,
    detection_wifi_disable_bit, is_ble_method_disabled, is_mobile_surveillance_ble_method,
    is_mobile_surveillance_method, is_mobile_surveillance_wifi_method,
    is_remote_id_ble_method_disabled, is_remote_id_wifi_method_disabled, is_wifi_method_disabled,
    remote_id_ble_disable_bit, remote_id_wifi_disable_bit, wifi_mask_needs_ie_sig,
    wifi_mask_needs_ms_oui, wifi_mask_needs_wildcard_probe, wifi_method_caps, MethodCaps,
    DETECTION_BLE_DISABLE_MASK_DEFAULT, DETECTION_WIFI_DISABLE_MASK_DEFAULT,
    MOBILE_SURVEILLANCE_AXON_METHOD_ID, MOBILE_SURVEILLANCE_AXON_WIFI_METHOD_ID,
    MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID, MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID,
};

#[test]
fn lpr_camera_wifi_disable_bit_method_1() {
    assert_eq!(detection_wifi_disable_bit(1), 1u32 << 0);
}

#[test]
fn lpr_camera_wifi_disable_bit_method_31() {
    assert_eq!(detection_wifi_disable_bit(31), 1u32 << 30);
}

#[test]
fn lpr_camera_wifi_disable_bit_method_0_returns_zero() {
    assert_eq!(detection_wifi_disable_bit(0), 0);
}

#[test]
fn lpr_camera_wifi_disable_bit_method_32_returns_zero() {
    assert_eq!(detection_wifi_disable_bit(32), 0);
}

#[test]
fn lpr_camera_wifi_disable_bit_method_255_returns_zero() {
    assert_eq!(detection_wifi_disable_bit(255), 0);
}

#[test]
fn lpr_camera_ble_disable_bit_method_32() {
    assert_eq!(detection_ble_disable_bit(32), 1u32 << 0);
}

#[test]
fn lpr_camera_ble_disable_bit_method_63() {
    assert_eq!(detection_ble_disable_bit(63), 1u32 << 31);
}

#[test]
fn lpr_camera_ble_disable_bit_method_31_returns_zero() {
    assert_eq!(detection_ble_disable_bit(31), 0);
}

#[test]
fn lpr_camera_ble_disable_bit_method_64_returns_zero() {
    assert_eq!(detection_ble_disable_bit(64), 0);
}

#[test]
fn is_wifi_method_disabled_mask_0_nothing_disabled() {
    for m in 1..=31 {
        assert!(
            !is_wifi_method_disabled(0, m),
            "method {} should be enabled with mask 0",
            m
        );
    }
}

#[test]
fn is_wifi_method_disabled_all_bits_all_disabled() {
    for m in 1..=31 {
        assert!(
            is_wifi_method_disabled(0xFFFFFFFF, m),
            "method {} should be disabled with all bits set",
            m
        );
    }
}

#[test]
fn is_wifi_method_disabled_single_bit() {
    let mask = detection_wifi_disable_bit(3);
    assert!(is_wifi_method_disabled(mask, 3));
    assert!(!is_wifi_method_disabled(mask, 1));
    assert!(!is_wifi_method_disabled(mask, 5));
}

#[test]
fn is_ble_method_disabled_mask_0_nothing_disabled() {
    for m in 32..=36 {
        assert!(
            !is_ble_method_disabled(0, m),
            "method {} should be enabled with mask 0",
            m
        );
    }
}

#[test]
fn is_ble_method_disabled_all_bits_all_disabled() {
    for m in 32..=36 {
        assert!(
            is_ble_method_disabled(0xFFFFFFFF, m),
            "method {} should be disabled with all bits set",
            m
        );
    }
}

#[test]
fn lpr_camera_wifi_default_mask_disables_method_1() {
    assert!(is_wifi_method_disabled(
        DETECTION_WIFI_DISABLE_MASK_DEFAULT,
        1
    ));
    assert!(!is_wifi_method_disabled(
        DETECTION_WIFI_DISABLE_MASK_DEFAULT,
        2
    ));
    assert!(!is_wifi_method_disabled(
        DETECTION_WIFI_DISABLE_MASK_DEFAULT,
        4
    ));
    assert!(!is_wifi_method_disabled(
        DETECTION_WIFI_DISABLE_MASK_DEFAULT,
        5
    ));
}

#[test]
fn lpr_camera_ble_default_mask_disables_32_33_34() {
    assert!(is_ble_method_disabled(
        DETECTION_BLE_DISABLE_MASK_DEFAULT,
        32
    ));
    assert!(is_ble_method_disabled(
        DETECTION_BLE_DISABLE_MASK_DEFAULT,
        33
    ));
    assert!(is_ble_method_disabled(
        DETECTION_BLE_DISABLE_MASK_DEFAULT,
        34
    ));
    assert!(!is_ble_method_disabled(
        DETECTION_BLE_DISABLE_MASK_DEFAULT,
        35
    )); // Axon on
    assert!(!is_ble_method_disabled(
        DETECTION_BLE_DISABLE_MASK_DEFAULT,
        36
    )); // i-PRO on
}

#[test]
fn wifi_mask_needs_wildcard_probe_disabled_returns_false() {
    assert!(!wifi_mask_needs_wildcard_probe(0xFFFFFFFF));
}

#[test]
fn wifi_mask_needs_wildcard_probe_method_1_enabled() {
    assert!(wifi_mask_needs_wildcard_probe(0));
}

#[test]
fn wifi_mask_needs_wildcard_probe_method_2_only() {
    let mask = detection_wifi_disable_bit(1) | detection_wifi_disable_bit(3);
    // method 2 is enabled and needs wildcard probe
    assert!(wifi_mask_needs_wildcard_probe(mask));
}

#[test]
fn wifi_mask_needs_wildcard_probe_methods_1_2_3_disabled() {
    let mask = detection_wifi_disable_bit(1)
        | detection_wifi_disable_bit(2)
        | detection_wifi_disable_bit(3);
    assert!(!wifi_mask_needs_wildcard_probe(mask));
}

#[test]
fn wifi_mask_needs_ie_sig_disabled_returns_false() {
    assert!(!wifi_mask_needs_ie_sig(0xFFFFFFFF));
}

#[test]
fn wifi_mask_needs_ie_sig_methods_2_3_enabled() {
    assert!(wifi_mask_needs_ie_sig(0));
}

#[test]
fn wifi_mask_needs_ie_sig_methods_2_3_disabled() {
    let mask = detection_wifi_disable_bit(2) | detection_wifi_disable_bit(3);
    assert!(!wifi_mask_needs_ie_sig(mask));
}

#[test]
fn wifi_mask_needs_ms_oui_all_disabled_returns_false() {
    let mask = detection_wifi_disable_bit(4) | detection_wifi_disable_bit(5);
    assert!(!wifi_mask_needs_ms_oui(mask));
}

#[test]
fn wifi_mask_needs_ms_oui_default_mask_returns_true() {
    assert!(wifi_mask_needs_ms_oui(DETECTION_WIFI_DISABLE_MASK_DEFAULT));
}

#[test]
fn wifi_mask_needs_ms_oui_method_4_only_disabled() {
    let mask = detection_wifi_disable_bit(4);
    assert!(wifi_mask_needs_ms_oui(mask)); // method 5 still enabled
}

#[test]
fn ble_mask_needs_detection_all_disabled_returns_false() {
    let mask = detection_ble_disable_bit(32)
        | detection_ble_disable_bit(33)
        | detection_ble_disable_bit(34)
        | detection_ble_disable_bit(35)
        | detection_ble_disable_bit(36);
    assert!(!ble_mask_needs_detection(mask));
}

#[test]
fn ble_mask_needs_detection_any_enabled_returns_true() {
    assert!(ble_mask_needs_detection(0));
}

#[test]
fn ble_mask_needs_detection_default_mask_returns_true() {
    assert!(ble_mask_needs_detection(DETECTION_BLE_DISABLE_MASK_DEFAULT));
}

#[test]
fn remote_id_wifi_disable_bit_method_64() {
    assert_eq!(remote_id_wifi_disable_bit(64), 1u32 << 0);
}

#[test]
fn remote_id_wifi_disable_bit_method_95() {
    assert_eq!(remote_id_wifi_disable_bit(95), 1u32 << 31);
}

#[test]
fn remote_id_wifi_disable_bit_out_of_range_returns_zero() {
    assert_eq!(remote_id_wifi_disable_bit(63), 0);
    assert_eq!(remote_id_wifi_disable_bit(96), 0);
    assert_eq!(remote_id_wifi_disable_bit(0), 0);
    assert_eq!(remote_id_wifi_disable_bit(255), 0);
}

#[test]
fn remote_id_ble_disable_bit_method_96() {
    assert_eq!(remote_id_ble_disable_bit(96), 1u32 << 0);
}

#[test]
fn remote_id_ble_disable_bit_method_127() {
    assert_eq!(remote_id_ble_disable_bit(127), 1u32 << 31);
}

#[test]
fn remote_id_ble_disable_bit_out_of_range_returns_zero() {
    assert_eq!(remote_id_ble_disable_bit(95), 0);
    assert_eq!(remote_id_ble_disable_bit(128), 0);
    assert_eq!(remote_id_ble_disable_bit(0), 0);
    assert_eq!(remote_id_ble_disable_bit(255), 0);
}

#[test]
fn is_remote_id_wifi_method_disabled_mask_0() {
    for m in 64..=95 {
        assert!(
            !is_remote_id_wifi_method_disabled(0, m),
            "method {} should be enabled",
            m
        );
    }
}

#[test]
fn is_remote_id_wifi_method_disabled_single_bit() {
    let mask = remote_id_wifi_disable_bit(64);
    assert!(is_remote_id_wifi_method_disabled(mask, 64));
    assert!(!is_remote_id_wifi_method_disabled(mask, 65));
}

#[test]
fn is_remote_id_ble_method_disabled_mask_0() {
    for m in 96..=127 {
        assert!(
            !is_remote_id_ble_method_disabled(0, m),
            "method {} should be enabled",
            m
        );
    }
}

#[test]
fn is_remote_id_ble_method_disabled_single_bit() {
    let mask = remote_id_ble_disable_bit(96);
    assert!(is_remote_id_ble_method_disabled(mask, 96));
    assert!(!is_remote_id_ble_method_disabled(mask, 97));
}

#[test]
fn is_mobile_surveillance_method_wifi_axon() {
    assert!(is_mobile_surveillance_method(4));
}

#[test]
fn is_mobile_surveillance_method_wifi_ipro() {
    assert!(is_mobile_surveillance_method(5));
}

#[test]
fn is_mobile_surveillance_method_ble_axon() {
    assert!(is_mobile_surveillance_method(35));
}

#[test]
fn is_mobile_surveillance_method_ble_ipro() {
    assert!(is_mobile_surveillance_method(36));
}

#[test]
fn is_mobile_surveillance_method_non_surveillance_returns_false() {
    assert!(!is_mobile_surveillance_method(1));
    assert!(!is_mobile_surveillance_method(2));
    assert!(!is_mobile_surveillance_method(3));
    assert!(!is_mobile_surveillance_method(32));
    assert!(!is_mobile_surveillance_method(33));
    assert!(!is_mobile_surveillance_method(34));
    assert!(!is_mobile_surveillance_method(0));
    assert!(!is_mobile_surveillance_method(64));
}

#[test]
fn is_mobile_surveillance_ble_method_axon_ipro_only() {
    assert!(is_mobile_surveillance_ble_method(35));
    assert!(is_mobile_surveillance_ble_method(36));
    assert!(!is_mobile_surveillance_ble_method(32));
    assert!(!is_mobile_surveillance_ble_method(33));
    assert!(!is_mobile_surveillance_ble_method(34));
    assert!(!is_mobile_surveillance_ble_method(4));
    assert!(!is_mobile_surveillance_ble_method(5));
    assert!(!is_mobile_surveillance_ble_method(0));
}

#[test]
fn is_mobile_surveillance_wifi_method_axon_ipro_only() {
    assert!(is_mobile_surveillance_wifi_method(4));
    assert!(is_mobile_surveillance_wifi_method(5));
    assert!(!is_mobile_surveillance_wifi_method(1));
    assert!(!is_mobile_surveillance_wifi_method(2));
    assert!(!is_mobile_surveillance_wifi_method(3));
    assert!(!is_mobile_surveillance_wifi_method(35));
    assert!(!is_mobile_surveillance_wifi_method(36));
}

#[test]
fn method_caps_wifi_requires_wildcard_probe() {
    let c = wifi_method_caps(1);
    assert!(c.requires_wildcard_probe);
    assert!(!c.is_ble);
}

#[test]
fn method_caps_wifi_does_not_require_wildcard_probe() {
    let c = wifi_method_caps(4);
    assert!(!c.requires_wildcard_probe);
    assert!(!c.is_ble);
}

#[test]
fn method_caps_ble_all_are_ble() {
    for m in 32..=36 {
        let c = ble_method_caps(m);
        assert!(c.is_ble, "method {} should be BLE", m);
        assert!(
            !c.requires_wildcard_probe,
            "method {} should not require wildcard probe",
            m
        );
    }
}

#[test]
fn method_caps_unknown_wifi_returns_all_false() {
    let c = wifi_method_caps(99);
    assert!(!c.requires_wildcard_probe);
    assert!(!c.requires_ie_sig);
    assert!(!c.is_ble);
}

#[test]
fn method_caps_unknown_ble_returns_all_false() {
    let c = ble_method_caps(99);
    assert!(!c.requires_wildcard_probe);
    assert!(!c.requires_ie_sig);
    assert!(!c.is_ble);
}

#[test]
fn mobile_surveillance_constants_match_expected_values() {
    assert_eq!(MOBILE_SURVEILLANCE_AXON_WIFI_METHOD_ID, 4);
    assert_eq!(MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID, 5);
    assert_eq!(MOBILE_SURVEILLANCE_AXON_METHOD_ID, 35);
    assert_eq!(MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID, 36);
}
