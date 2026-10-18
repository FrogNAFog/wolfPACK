//! Remote ID immediate alert (`FrameType::RemoteIdAlert`) ESP-NOW handler.

use std::sync::Arc;

use wardriver_core::detection::{
    is_remote_id_ble_method_disabled, is_remote_id_wifi_method_disabled, DetectionSignalKind,
};
use wardriver_proto::{self as proto, RemoteIdAlertV1};
use wardriver_stats as stats;

use crate::csv;
use crate::remote_id_bridge::{
    pack_remote_id_csv_row, remote_id_last_from_alert, RemoteIdCsvScratch,
};
use crate::time_sync;
use crate::wifi_scan;

use super::csv_ingest::{choose_fix_from_history, fix_to_geo};
use super::ingress_common::alert_prologue;
use super::ingress_ctx::ControlIngressCtx;
use wardriver_core::gps_anchor::{observed_unix_ms_from_anchor, MAX_FIX_TIMELINE_DELTA_MS};

pub(super) fn handle_remote_id_alert(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let Some(alert) = alert_prologue(
        ctx,
        peer,
        data,
        &stats::REMOTE_ID_ALERTS_RX,
        ctx.remote_id_dis,
        |mask, a| {
            if a.signal_kind == DetectionSignalKind::Wifi.as_u8() {
                is_remote_id_wifi_method_disabled(mask, a.detection_method)
            } else {
                is_remote_id_ble_method_disabled(mask, a.detection_method)
            }
        },
        false,
        proto::unpack_remote_id_alert,
        |a| a.sniffer_id,
        |a| a.sniffer_boot_id,
        |a| &a.src,
        |_| {},
        |_, _| false,
    ) else {
        return;
    };
    if ctx.remote_id_alert_log.contains_mac(&alert.src) {
        handle_remote_id_followup(ctx, &alert);
        return;
    }
    handle_remote_id_alert_inner(ctx, &alert);
}

struct ObserverFix {
    lat: Option<f64>,
    lon: Option<f64>,
    altitude_m: Option<i32>,
    accuracy_m: Option<f32>,
    ts: Option<Arc<str>>,
}

fn observer_fix_for_alert(ctx: &ControlIngressCtx<'_>, alert: &RemoteIdAlertV1) -> ObserverFix {
    let fix_snapshot = ctx.gps_fix.lock().ok().and_then(|g| g.clone());
    let obs_unix = observed_unix_ms_from_anchor(
        alert.sniffer_scan_end_ms,
        alert.sniffer_scan_end_unix_ms,
        alert.sniffer_scan_end_ms,
    );
    let fix_hist = obs_unix.and_then(|t| {
        choose_fix_from_history(ctx.gps_hist, t, MAX_FIX_TIMELINE_DELTA_MS)
            .filter(|f| wifi_scan::is_fix_usable_for_location(f))
    });
    let fix_use = fix_hist.or(fix_snapshot);
    let (lat, lon, altitude_m, accuracy_m) = match fix_use
        .as_ref()
        .filter(|f| wifi_scan::is_fix_usable_for_location(f))
    {
        Some(f) => fix_to_geo(f),
        None => (None, None, None, None),
    };
    let ts = obs_unix
        .and_then(|t| time_sync::utc_string_from_unix_ms(t))
        .or_else(|| time_sync::now_utc_string())
        .map(Arc::from);
    ObserverFix {
        lat,
        lon,
        altitude_m,
        accuracy_m,
        ts,
    }
}

fn enqueue_remote_id_row(ctx: &ControlIngressCtx<'_>, alert: &RemoteIdAlertV1, obs: &ObserverFix) {
    if !stats::csv_enqueue_allowed() {
        return;
    }
    let Some(ts) = obs.ts.as_ref() else {
        return;
    };
    let mut uas_id_buf = String::new();
    let mut operator_id_buf = String::new();
    let mut id_type_buf = String::new();
    let mut ua_type_buf = String::new();
    let mut status_buf = String::new();
    let mut operator_location_type_buf = String::new();
    let mut operator_id_type_buf = String::new();
    let mut scratch = RemoteIdCsvScratch {
        uas_id: &mut uas_id_buf,
        operator_id: &mut operator_id_buf,
        id_type: &mut id_type_buf,
        ua_type: &mut ua_type_buf,
        status: &mut status_buf,
        operator_location_type: &mut operator_location_type_buf,
        operator_id_type: &mut operator_id_type_buf,
    };
    let drow = pack_remote_id_csv_row(
        alert,
        ts.as_ref(),
        obs.lat,
        obs.lon,
        obs.altitude_m,
        obs.accuracy_m,
        &mut scratch,
    );
    let mut line = Vec::<u8>::with_capacity(512);
    if csv::write_pack_remote_id_row(&mut line, &drow).is_ok() {
        let _ = ctx.sd_io.try_enqueue_remote_id_row(line);
    }
}

fn handle_remote_id_followup(ctx: &mut ControlIngressCtx<'_>, alert: &RemoteIdAlertV1) {
    let obs = observer_fix_for_alert(ctx, alert);
    let (Some(lat), Some(lon)) = (obs.lat, obs.lon) else {
        return;
    };
    if ctx
        .remote_id_alert_log
        .try_followup(&alert.src, lat, lon, ctx.mac_table)
        .is_none()
    {
        return;
    }
    enqueue_remote_id_row(ctx, alert, &obs);
}

fn handle_remote_id_alert_inner(ctx: &mut ControlIngressCtx<'_>, alert: &RemoteIdAlertV1) {
    let obs = observer_fix_for_alert(ctx, alert);
    if let (Some(lat), Some(lon)) = (obs.lat, obs.lon) {
        let _ = ctx.mac_table.allow_remote_id(&alert.src, lat, lon);
    }

    stats::record_remote_id_alert(remote_id_last_from_alert(ctx.now_ms, alert));
    ctx.remote_id_alert_log
        .remember(alert.src, alert.detection_method, alert.signal_kind);
    ctx.alert_hub
        .trigger(crate::alert_feedback::AlertKind::RemoteId, None);

    enqueue_remote_id_row(ctx, alert, &obs);
}
