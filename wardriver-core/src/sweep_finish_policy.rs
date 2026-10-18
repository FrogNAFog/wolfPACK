//! Pure sweep-finish policy helpers (host-testable; no esp-idf / stats).

use wardriver_proto::DetectionBleAlertV1;

use crate::lpr_camera_ble::BleCameraAlert;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HopResumeInput {
    pub aborted_due_to_pressure: bool,
    pub ended_max_off_break: bool,
    pub current_channel_idx: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HopResumeAction {
    KeepIndex,
    ResetIndexToZero,
    ClearAbortPressureFlag,
}

pub fn hop_resume_action(input: HopResumeInput) -> HopResumeAction {
    if input.aborted_due_to_pressure {
        HopResumeAction::KeepIndex
    } else if input.ended_max_off_break {
        HopResumeAction::ClearAbortPressureFlag
    } else {
        HopResumeAction::ResetIndexToZero
    }
}

/// WiFi: increment only on scan_only normal finish, not alert path.
pub fn should_count_wardrive_suppressed_skip(scan_only: bool, has_detection_alerts: bool) -> bool {
    scan_only && !has_detection_alerts
}

pub fn ble_detection_alert_v1_from(
    alert: &BleCameraAlert,
    sniffer_id: u32,
    seq: u32,
    sniffer_boot_id: u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
) -> DetectionBleAlertV1 {
    DetectionBleAlertV1 {
        sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms: scan_end_ms,
        sniffer_scan_end_unix_ms: scan_end_unix_ms,
        src: alert.src,
        detection_method: alert.method,
        company_id: alert.company_id,
        rssi: alert.rssi,
    }
}
