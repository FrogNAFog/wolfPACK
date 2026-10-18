//! Host tests for sweep-finish policy helpers.

use wardriver_core::lpr_camera_ble::BleCameraAlert;
use wardriver_core::sweep_finish_policy::{
    ble_detection_alert_v1_from, hop_resume_action, should_count_wardrive_suppressed_skip,
    HopResumeAction, HopResumeInput,
};

#[test]
fn hop_resume_pressure_abort_keeps_index() {
    let action = hop_resume_action(HopResumeInput {
        aborted_due_to_pressure: true,
        ended_max_off_break: false,
        current_channel_idx: 4,
    });
    assert_eq!(action, HopResumeAction::KeepIndex);
}

#[test]
fn hop_resume_max_off_break_clears_pressure_flag_only() {
    let action = hop_resume_action(HopResumeInput {
        aborted_due_to_pressure: false,
        ended_max_off_break: true,
        current_channel_idx: 4,
    });
    assert_eq!(action, HopResumeAction::ClearAbortPressureFlag);
}

#[test]
fn hop_resume_normal_sweep_resets_index() {
    let action = hop_resume_action(HopResumeInput {
        aborted_due_to_pressure: false,
        ended_max_off_break: false,
        current_channel_idx: 4,
    });
    assert_eq!(action, HopResumeAction::ResetIndexToZero);
}

#[test]
fn wardrive_suppressed_skip_only_on_scan_only_without_alerts() {
    assert!(should_count_wardrive_suppressed_skip(true, false));
    assert!(!should_count_wardrive_suppressed_skip(true, true));
    assert!(!should_count_wardrive_suppressed_skip(false, false));
    assert!(!should_count_wardrive_suppressed_skip(false, true));
}

#[test]
fn ble_detection_alert_v1_from_maps_fields() {
    let alert = BleCameraAlert {
        sniffer_id: 99,
        src: [0x70, 0xC9, 0x4E, 0x01, 0x02, 0x03],
        rssi: -62,
        company_id: 0x09C8,
        method: 32,
    };
    let wire = ble_detection_alert_v1_from(&alert, 7, 42, 0xBEEF, 1_000, 1_700_000_000_000);
    assert_eq!(wire.sniffer_id, 7);
    assert_eq!(wire.seq, 42);
    assert_eq!(wire.sniffer_boot_id, 0xBEEF);
    assert_eq!(wire.sniffer_scan_end_ms, 1_000);
    assert_eq!(wire.sniffer_scan_end_unix_ms, 1_700_000_000_000);
    assert_eq!(wire.src, alert.src);
    assert_eq!(wire.detection_method, 32);
    assert_eq!(wire.company_id, 0x09C8);
    assert_eq!(wire.rssi, -62);
}
