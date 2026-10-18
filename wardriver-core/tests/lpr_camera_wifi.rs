//! Host tests for lpr_camera_wifi edge cases: OUI allowlist, cooldown, method eval.

use wardriver_core::detection::{
    is_wifi_method_disabled, DetectionWifiMethod, DETECTION_WIFI_DISABLE_MASK_DEFAULT,
};
use wardriver_core::lpr_camera_wifi::{
    CameraProbeSample, LprCameraWifiDetector, LPR_CAMERA_OUIS, PER_SRC_COOLDOWN_MS,
};

/// A MAC from a well-known consumer router manufacturer NOT in the LPR camera OUI allowlist.
/// (TP-Link: 7C:9E:1X)
const NON_FLOCK_MAC: [u8; 6] = [0x7C, 0x9E, 0x14, 0x00, 0x00, 0x01];

/// Another non-camera consumer brand (D-Link: 00:0C:4X)
const NON_FLOCK_MAC2: [u8; 6] = [0x00, 0x0C, 0x4B, 0x00, 0x00, 0x02];

/// Generic IoT device OUI not in allowlist
const NON_FLOCK_MAC3: [u8; 6] = [0xA0, 0xA2, 0xA3, 0x00, 0x00, 0x03];

/// False negative: non-camera MAC should NOT trigger method 1 (wildcard + OUI)
#[test]
fn lpr_camera_oui_false_negative_tp_link() {
    let mut det = LprCameraWifiDetector::new();
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC,
        wildcard_count: 150,
        channel: 6,
        rssi_min: -70,
        rssi_max: -55,
        rssi_avg_q8: (-60 << 8),
        primary_sig_matched: true,
    };
    let outcome = det.eval_burst(1, 1000, &sample, 0);
    assert!(
        outcome.method1.is_none(),
        "non-camera OUI should not trigger method 1"
    );
    // Method 3 (IE sig any MAC) should fire if primary_sig_matched
    assert!(
        outcome.method3.is_some(),
        "method 3 should fire on IE match regardless of OUI"
    );
}

/// False negative: D-Link brand MAC not in LPR camera allowlist
#[test]
fn lpr_camera_oui_false_negative_dlink() {
    let mut det = LprCameraWifiDetector::new();
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC2,
        wildcard_count: 200,
        channel: 11,
        rssi_min: -65,
        rssi_max: -50,
        rssi_avg_q8: (-58 << 8),
        primary_sig_matched: true,
    };
    let outcome = det.eval_burst(1, 2000, &sample, 0);
    assert!(outcome.method1.is_none());
    assert!(outcome.method3.is_some());
}

/// False negative: generic IoT MAC with high wildcard count
#[test]
fn lpr_camera_oui_false_negative_generic_iot() {
    let mut det = LprCameraWifiDetector::new();
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC3,
        wildcard_count: 255,
        channel: 1,
        rssi_min: -80,
        rssi_max: -60,
        rssi_avg_q8: (-70 << 8),
        primary_sig_matched: false,
    };
    let outcome = det.eval_burst(1, 3000, &sample, 0);
    assert!(
        outcome.method1.is_none(),
        "non-camera OUI + no IE sig = no alert"
    );
    assert!(outcome.method2.is_none());
    assert!(
        outcome.method3.is_none(),
        "no IE sig = method 3 should not fire"
    );
}

/// High wildcard count alone on a consumer MAC does NOT trigger method 1
#[test]
fn high_wildcard_on_consumer_mac_no_false_positive() {
    let mut det = LprCameraWifiDetector::new();
    // Even with max wildcard count and IE signature match
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC,
        wildcard_count: 255,
        channel: 6,
        rssi_min: -40,
        rssi_max: -30,
        rssi_avg_q8: (-35 << 8),
        primary_sig_matched: true, // strong signal, but wrong OUI
    };
    let outcome = det.eval_burst(1, 5000, &sample, 0);
    assert!(
        outcome.method1.is_none(),
        "OUI check blocks method 1 even with high wildcard"
    );
    assert!(outcome.method2.is_none(), "method 2 requires OUI + IE sig");
}

/// Method 3 fires on IE signature match regardless of MAC OUI
#[test]
fn method3_fires_on_ie_sig_any_mac() {
    let mut det = LprCameraWifiDetector::new();
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC,
        wildcard_count: 50,
        channel: 1,
        rssi_min: -75,
        rssi_max: -60,
        rssi_avg_q8: (-68 << 8),
        primary_sig_matched: true,
    };
    let outcome = det.eval_burst(1, 1000, &sample, 0);
    assert!(outcome.method3.is_some(), "method 3 does not check OUI");
}

/// Method 2 requires BOTH OUI match AND IE signature
#[test]
fn method2_requires_oui_and_ie_sig() {
    // Non-camera MAC with IE sig — method 2 should NOT fire
    let mut det = LprCameraWifiDetector::new();
    let sample = CameraProbeSample {
        src: NON_FLOCK_MAC,
        wildcard_count: 100,
        channel: 6,
        rssi_min: -65,
        rssi_max: -50,
        rssi_avg_q8: (-58 << 8),
        primary_sig_matched: true,
    };
    let outcome = det.eval_burst(1, 1000, &sample, 0);
    assert!(
        outcome.method2.is_none(),
        "method 2 requires camera OUI + IE sig"
    );
}

/// Verify all OUIs in the allowlist actually trigger method 1
#[test]
fn all_lpr_camera_ouis_trigger_method1() {
    for oui in LPR_CAMERA_OUIS {
        let mut det = LprCameraWifiDetector::new();
        let mut mac = [0u8; 6];
        mac[..3].copy_from_slice(oui);
        let sample = CameraProbeSample {
            src: mac,
            wildcard_count: 100,
            channel: 6,
            rssi_min: -65,
            rssi_max: -50,
            rssi_avg_q8: (-58 << 8),
            primary_sig_matched: true,
        };
        let outcome = det.eval_burst(1, 1000, &sample, 0);
        assert!(
            outcome.method1.is_some(),
            "OUI {:02X}:{:02X}:{:02X} should trigger method 1",
            oui[0],
            oui[1],
            oui[2]
        );
    }
}

/// Zero wildcard count produces no alerts
#[test]
fn zero_wildcard_count_no_alert() {
    let mut det = LprCameraWifiDetector::new();
    let mac = {
        let mut m = [0u8; 6];
        m[..3].copy_from_slice(&LPR_CAMERA_OUIS[0]);
        m
    };
    let sample = CameraProbeSample {
        src: mac,
        wildcard_count: 0,
        channel: 6,
        rssi_min: -65,
        rssi_max: -50,
        rssi_avg_q8: (-58 << 8),
        primary_sig_matched: true,
    };
    let outcome = det.eval_burst(1, 1000, &sample, 0);
    assert!(outcome.method1.is_none());
    assert!(outcome.method2.is_none());
    assert!(outcome.method3.is_none());
}

/// Cooldown: repeat probe from same camera MAC within 30s is suppressed
#[test]
fn cooldown_suppresses_repeat_probe() {
    let mut det = LprCameraWifiDetector::new();
    let mac = {
        let mut m = [0u8; 6];
        m[..3].copy_from_slice(&LPR_CAMERA_OUIS[0]);
        m
    };

    let sample = CameraProbeSample {
        src: mac,
        wildcard_count: 100,
        channel: 6,
        rssi_min: -65,
        rssi_max: -50,
        rssi_avg_q8: (-58 << 8),
        primary_sig_matched: true,
    };

    let first = det.eval_burst(1, 1000, &sample, 0);
    assert!(first.method1.is_some());

    // Same MAC, 15s later — should be suppressed
    let second = det.eval_burst(1, 1000 + 15_000, &sample, 0);
    assert!(
        second.method1.is_none(),
        "cooldown should suppress within 30s"
    );

    // Same MAC, 31s later — should fire
    let third = det.eval_burst(1, 1000 + 31_000, &sample, 0);
    assert!(
        third.method1.is_some(),
        "should fire after cooldown expires"
    );
}

/// Cooldown constant is 30 seconds
#[test]
fn cooldown_constant_is_30s() {
    assert_eq!(PER_SRC_COOLDOWN_MS, 30_000);
}

/// eval_mac: Axon OUI triggers method 4 (mobile surveillance)
#[test]
fn eval_mac_axon_oui_triggers_method4() {
    let mut det = LprCameraWifiDetector::new();
    // Axon OUI: 00:25:DF:xx:xx:xx
    let mac = [0x00, 0x25, 0xDF, 0x00, 0x00, 0x01];
    let alert = det.eval_mac(1, 1000, mac, 6, -60, 0, false);
    assert!(alert.is_some());
    assert_eq!(alert.unwrap().wifi_method, DetectionWifiMethod::AxonOui);
}

/// eval_mac: i-PRO OUI triggers method 5 only when allow_ipro is true
#[test]
fn eval_mac_ipro_requires_allow_ipro() {
    let mut det = LprCameraWifiDetector::new();
    // i-PRO OUI: D4:2D:C5:xx:xx:xx
    let mac = [0xD4, 0x2D, 0xC5, 0x00, 0x00, 0x01];

    // Should NOT fire when allow_ipro is false
    let alert = det.eval_mac(1, 1000, mac, 6, -60, 0, false);
    assert!(
        alert.is_none(),
        "i-PRO should be suppressed without allow_ipro"
    );

    // Should fire when allow_ipro is true
    let alert = det.eval_mac(1, 1000, mac, 6, -60, 0, true);
    assert!(alert.is_some());
    assert_eq!(alert.unwrap().wifi_method, DetectionWifiMethod::IproOui);
}

/// eval_mac: non-matching OUI returns None
#[test]
fn eval_mac_non_matching_oui_returns_none() {
    let mut det = LprCameraWifiDetector::new();
    let mac = NON_FLOCK_MAC;
    let alert = det.eval_mac(1, 1000, mac, 6, -60, 0, true);
    assert!(
        alert.is_none(),
        "non-Axon/non-iPRO OUI should not trigger eval_mac"
    );
}

/// Disable mask blocks method 4 (Axon) when bit is set
#[test]
fn eval_mac_disable_mask_blocks_axon() {
    let mut det = LprCameraWifiDetector::new();
    // Axon OUI: 00:25:DF:xx:xx:xx
    let mac = [0x00, 0x25, 0xDF, 0x00, 0x00, 0x01];
    // Disable method 4 (Axon OUI) — bit = method - 1 = 3
    let mask = 1u32 << 3;
    let alert = det.eval_mac(1, 1000, mac, 6, -60, mask, false);
    assert!(
        alert.is_none(),
        "Axon OUI should be blocked by disable mask"
    );
}

/// Disable mask with DETECTION_WIFI constant blocks method 1
#[test]
fn detection_wifi_disable_mask_blocks_method1() {
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
        3
    ));
}

/// mac_matches_lpr_camera_oui works for allowlisted OUIs
#[test]
fn mac_matches_infrastructure_oui_for_allowlisted() {
    use wardriver_core::lpr_camera_wifi::mac_matches_lpr_camera_oui;
    for oui in LPR_CAMERA_OUIS {
        let mut mac = [0u8; 6];
        mac[..3].copy_from_slice(oui);
        assert!(
            mac_matches_lpr_camera_oui(&mac),
            "OUI {:02X}:{:02X}:{:02X} should match",
            oui[0],
            oui[1],
            oui[2]
        );
    }
}

/// mac_matches_lpr_camera_oui rejects non-camera MACs
#[test]
fn mac_matches_infrastructure_oui_rejects_non_camera() {
    use wardriver_core::lpr_camera_wifi::mac_matches_lpr_camera_oui;
    assert!(
        !mac_matches_lpr_camera_oui(&NON_FLOCK_MAC),
        "TP-Link MAC should not match camera OUI"
    );
}

/// Prune removes stale entries after 60s idle
#[test]
fn prune_removes_stale_entries() {
    let mut det = LprCameraWifiDetector::new();
    let mac = {
        let mut m = [0u8; 6];
        m[..3].copy_from_slice(&LPR_CAMERA_OUIS[0]);
        m
    };
    let sample = CameraProbeSample {
        src: mac,
        wildcard_count: 100,
        channel: 6,
        rssi_min: -65,
        rssi_max: -50,
        rssi_avg_q8: (-58 << 8),
        primary_sig_matched: true,
    };
    det.eval_burst(1, 1000, &sample, 0);
    // last_seen_ms = 1000, last_alert_ms[0] = 1000
    // Prune at 1000 + 59000 = 60000: 60000 - 1000 = 59000 <= 60000, so retained
    det.prune(60_000);

    // Entry still alive — cooldown allows (60000 - 1000 = 59000 >= 30000)
    let r = det.eval_burst(1, 60_000, &sample, 0);
    assert!(
        r.method1.is_some(),
        "should fire after 59s idle (cooldown expired)"
    );

    // eval_burst updated last_seen_ms to 60000 and last_alert_ms[0] to 60000
    // Prune at 60000 + 61000 = 121000: 121000 - 60000 = 61000 > 60000, so evicted
    det.prune(121_000);

    // Entry is gone — next burst creates fresh state with no cooldown
    let r = det.eval_burst(1, 121_000, &sample, 0);
    assert!(
        r.method1.is_some(),
        "should fire after prune (fresh state, no cooldown)"
    );
}
