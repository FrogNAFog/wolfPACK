//! WiFi LPR-camera / mobile-surveillance immediate alert (`FrameType::DetectionAlert`) ESP-NOW handler.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use wardriver_core::detection::{
    is_mobile_surveillance_wifi_method, is_wifi_method_disabled, DetectionSignalKind,
};
use wardriver_proto::{self as proto, DetectionAlertV1};
use wardriver_stats as stats;

use crate::csv;
use crate::time_sync;
use crate::wifi_scan;

use super::csv_ingest::{choose_fix_from_history, fix_to_geo};
use super::ingress_common::alert_prologue;
use super::ingress_ctx::ControlIngressCtx;
use crate::detection_wifi_bridge::detection_last_from_detection_alert;
use wardriver_core::gps_anchor::{observed_unix_ms_from_anchor, MAX_FIX_TIMELINE_DELTA_MS};

pub(super) fn handle_detection_alert(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let Some(alert) = alert_prologue(
        ctx,
        peer,
        data,
        &stats::DETECTION_ALERTS_RX,
        ctx.detection_dis_w,
        |mask, a| is_wifi_method_disabled(mask, a.detection_method),
        true,
        proto::unpack_detection_alert,
        |a| a.sniffer_id,
        |a| a.sniffer_boot_id,
        |a| &a.src,
        |a| {
            if a.primary_sig_matched != 0 {
                stats::DETECTION_WIFI_IE_SIG_MATCH_BUILTIN_TOTAL.fetch_add(1, Ordering::Relaxed);
            }
        },
        |c, a| c.detection_alert_log_wifi.contains_mac(&a.src),
    ) else {
        return;
    };
    handle_detection_wifi_alert(ctx, &alert);
}

fn handle_detection_wifi_alert(ctx: &mut ControlIngressCtx<'_>, alert: &DetectionAlertV1) {
    let detection_fix_snapshot = ctx.gps_fix.lock().ok().and_then(|g| g.clone());
    let obs_unix = observed_unix_ms_from_anchor(
        alert.sniffer_scan_end_ms,
        alert.sniffer_scan_end_unix_ms,
        alert.first_seen_ms32,
    );
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
    let is_ms = is_mobile_surveillance_wifi_method(alert.detection_method);

    if let Some(record) = crate::detection_wifi_bridge::detection_alert_v1_to_record(alert) {
        *ctx.detection_last_alert = Some(record);
    }

    if is_ms {
        ctx.alert_hub.trigger_with_title(
            crate::alert_feedback::AlertKind::Surveillance,
            csv::mobile_surveillance_oled_title(alert.detection_method),
            None,
        );
        stats::record_mobile_surveillance_alert(stats::MobileSurveillanceLast {
            last_seen_ms: ctx.now_ms,
            sniffer_id: alert.sniffer_id,
            src: alert.src,
            signal_kind: DetectionSignalKind::Wifi.as_u8(),
            detection_method: alert.detection_method,
        });
    } else {
        stats::record_detection_alert(detection_last_from_detection_alert(ctx.now_ms, alert));
        stats::record_detection_wifi_method_alert(alert.detection_method);
        ctx.alert_hub
            .trigger(crate::alert_feedback::AlertKind::Camera, None);
    }

    ctx.detection_alert_log_wifi.remember(
        alert.src,
        alert.detection_method,
        DetectionSignalKind::Wifi.as_u8(),
        is_ms,
    );

    if csv_ok {
        if let Some(ts) = ts_arc.as_ref() {
            let rssi_avg = (alert.rssi_avg_q8 as f32) / 256.0;
            let ch = alert.channel;
            let (row_type, session) = csv::detection_session_for_method(
                alert.detection_method,
                DetectionSignalKind::Wifi.as_u8(),
            );
            let drow = csv::DetectionAlertCsvRow {
                row_type,
                detection_method_id: alert.detection_method,
                signal_kind: DetectionSignalKind::Wifi.as_u8(),
                first_seen_utc: ts.as_ref(),
                lat,
                lon,
                altitude_m,
                accuracy_m,
                mac: alert.src,
                ssid: "",
                auth_mode: "",
                channel: ch,
                frequency_mhz: csv::channel_to_frequency_mhz(ch),
                rssi: rssi_avg.round() as i8,
                rssi_min: alert.rssi_min,
                rssi_max: alert.rssi_max,
                rssi_avg_q8: alert.rssi_avg_q8,
                wildcard_count: u16::from(alert.wildcard_count),
                distinct_ch: u8::from(ch > 0),
                rcois: "",
                mfgr_id: "",
            };
            let _ = csv::enqueue_detection_session_row(ctx.sd_io, &drow, session);
        }
    }
}
