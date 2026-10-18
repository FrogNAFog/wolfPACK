//! BLE LPR-camera / mobile-surveillance immediate alert (`FrameType::DetectionBleAlert`) ESP-NOW handler.

use std::sync::Arc;

use wardriver_core::detection::{
    is_ble_method_disabled, is_mobile_surveillance_ble_method, DetectionSignalKind,
};
use wardriver_proto::{self as proto, DetectionBleAlertV1};
use wardriver_stats as stats;

use crate::csv;
use crate::time_sync;
use crate::wifi_scan;

use super::csv_ingest::{choose_fix_from_history, fix_to_geo};
use super::ingress_common::alert_prologue;
use super::ingress_ctx::ControlIngressCtx;
use wardriver_core::gps_anchor::MAX_FIX_TIMELINE_DELTA_MS;

pub(super) fn handle_detection_ble_alert(
    ctx: &mut ControlIngressCtx<'_>,
    peer: [u8; 6],
    data: &[u8],
) {
    let Some(alert) = alert_prologue(
        ctx,
        peer,
        data,
        &stats::DETECTION_ALERTS_RX,
        ctx.detection_dis_b,
        |mask, a| is_ble_method_disabled(mask, a.detection_method),
        true,
        proto::unpack_detection_ble_alert,
        |a| a.sniffer_id,
        |a| a.sniffer_boot_id,
        |a| &a.src,
        |_| {},
        |c, a| c.detection_alert_log_ble.contains_mac(&a.src),
    ) else {
        return;
    };
    handle_detection_ble_alert_inner(ctx, &alert);
}

fn handle_detection_ble_alert_inner(ctx: &mut ControlIngressCtx<'_>, alert: &DetectionBleAlertV1) {
    let detection_fix_snapshot = ctx.gps_fix.lock().ok().and_then(|g| g.clone());
    let obs_unix = if alert.sniffer_scan_end_unix_ms != 0 {
        Some(alert.sniffer_scan_end_unix_ms as i64)
    } else {
        None
    };
    let fix_hist = obs_unix.and_then(|t| {
        choose_fix_from_history(ctx.gps_hist, t, MAX_FIX_TIMELINE_DELTA_MS)
            .filter(|f| wifi_scan::is_fix_usable_for_location(f))
    });
    let fix_use = fix_hist.or(detection_fix_snapshot);
    let (lat, lon, altitude_m, accuracy_m) = match fix_use
        .as_ref()
        .filter(|f| wifi_scan::is_fix_usable_for_location(f))
    {
        Some(f) => fix_to_geo(f),
        None => (None, None, None, None),
    };
    let ts_s = obs_unix
        .and_then(|t| time_sync::utc_string_from_unix_ms(t))
        .or_else(|| time_sync::now_utc_string());
    let ts_arc: Option<Arc<str>> = ts_s.map(Arc::from);
    let csv_ok = stats::csv_enqueue_allowed();
    let now_ms = ctx.now_ms;
    let is_ms = is_mobile_surveillance_ble_method(alert.detection_method);

    if is_ms {
        ctx.alert_hub.trigger_with_title(
            crate::alert_feedback::AlertKind::Surveillance,
            csv::mobile_surveillance_oled_title(alert.detection_method),
            None,
        );
        stats::record_mobile_surveillance_alert(stats::MobileSurveillanceLast {
            last_seen_ms: now_ms,
            sniffer_id: alert.sniffer_id,
            src: alert.src,
            signal_kind: DetectionSignalKind::Ble.as_u8(),
            detection_method: alert.detection_method,
        });
    } else {
        ctx.alert_hub
            .trigger(crate::alert_feedback::AlertKind::Camera, None);
        let rssi_q8 = (alert.rssi as i16) << 8;
        stats::record_detection_alert(stats::DetectionLast {
            last_seen_ms: now_ms,
            sniffer_id: alert.sniffer_id,
            src: alert.src,
            wildcard_count: 0,
            channel: 0,
            rssi_min: alert.rssi,
            rssi_max: alert.rssi,
            rssi_avg_q8: rssi_q8,
            oui_matched: 0,
            signal_kind: DetectionSignalKind::Ble.as_u8(),
            detection_method: alert.detection_method,
            ble_company_id: alert.company_id,
        });
    }

    ctx.detection_alert_log_ble.remember(
        alert.src,
        alert.detection_method,
        DetectionSignalKind::Ble.as_u8(),
        is_ms,
    );

    if csv_ok {
        if let Some(ts) = ts_arc.as_ref() {
            let (row_type, session) = csv::detection_session_for_method(
                alert.detection_method,
                DetectionSignalKind::Ble.as_u8(),
            );
            let mfgr_id_owned;
            let mfgr_id: &str = if alert.company_id != 0 {
                mfgr_id_owned = alert.company_id.to_string();
                &mfgr_id_owned
            } else {
                ""
            };
            let drow = csv::DetectionAlertCsvRow {
                row_type,
                detection_method_id: alert.detection_method,
                signal_kind: DetectionSignalKind::Ble.as_u8(),
                first_seen_utc: ts.as_ref(),
                lat,
                lon,
                altitude_m,
                accuracy_m,
                mac: alert.src,
                ssid: "",
                auth_mode: "",
                channel: 0,
                frequency_mhz: 0,
                rssi: alert.rssi,
                rssi_min: alert.rssi,
                rssi_max: alert.rssi,
                rssi_avg_q8: (alert.rssi as i16) << 8,
                wildcard_count: 0,
                distinct_ch: 0,
                rcois: "",
                mfgr_id,
            };
            let _ = csv::enqueue_detection_session_row(ctx.sd_io, &drow, session);
        }
    }
}
