//! BLE-role scan tick: capture phase + shared sweep finish on the control channel.

use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::warn;
use wardriver_shared::spiram_queue::Receiver as EspnowRxReceiver;

use crate::ble_sniffer;
use crate::sniffer_dispatch;
use crate::sniffer_espnow::{sniffer_unix_now_ms, SnifferCtx};
use crate::sniffer_outbound_bulk::BulkDumpPlan;
use crate::sniffer_outbound_bulk::OutboundBulkStore;
use crate::sniffer_sweep_finish::{finish_sweep_on_control, SweepAlerts};
use crate::sniffer_wifi::{
    pause_wifi_for_ble_capture, resume_wifi_after_ble_capture, sniffer_boot_id,
};
use wardriver_core::lpr_camera_ble::BleCameraAlert;
use wardriver_proto::{self as proto, SnifferState};
use wardriver_shared::espnow;

struct BleCaptureOutcome {
    detection_alerts: Vec<BleCameraAlert>,
    remote_id_alerts: Vec<wardriver_proto::RemoteIdAlertV1>,
    had_bulk: bool,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
}

fn run_ble_capture_phase(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ctx: &mut SnifferCtx<'_>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    outbound: &mut OutboundBulkStore,
    control_ch: u8,
    seq: u32,
) -> Result<Option<BleCaptureOutcome>> {
    let scan_window_ms = proto::BLE_SCAN_WINDOW_MS.min(u32::MAX as u64) as u32;
    let scan_only = ctx.cfg.scan_only();

    if scan_only {
        outbound.clear();
    }

    ctx.drain_rx(rx_rx)?;
    if *ctx.state != SnifferState::Scanning {
        return Ok(None);
    }

    if scan_only {
        ble_sniffer::prune_camera_detector(wardriver_stats::now_ms());
    }

    if !pause_wifi_for_ble_capture(wifi) {
        warn!("sniffer: ble capture: wifi pause failed; running degraded (wifi on)");
    }

    let outcome = ble_sniffer::sweep_ble_window(
        ctx.sniffer_id,
        ctx.cfg.detection_disable_ble_mask,
        ctx.cfg.remote_id_disable_ble_mask,
        seq,
        sniffer_boot_id(),
        scan_window_ms,
        !scan_only,
        ctx.cfg.ble_active_scan,
    );

    if !resume_wifi_after_ble_capture(
        wifi,
        control_ch,
        (*ctx.control_mac).or(ctx.cfg.bound_control_mac),
    ) {
        warn!(
            "sniffer: BLE sweep: failed to resume wifi on control channel {control_ch}; skipping finish"
        );
        sniffer_dispatch::record_bulk_cycle(!outcome.wardrive_entries.is_empty(), scan_only, false);
        std::thread::sleep(Duration::from_millis(50));
        return Ok(None);
    }

    ctx.flush_pending_time_set();

    let scan_end_ms = wardriver_stats::now_ms() as u32;
    let scan_end_unix_ms = sniffer_unix_now_ms();
    let had_bulk = !outcome.wardrive_entries.is_empty();

    if !scan_only && had_bulk {
        outbound.insert_ble(
            scan_end_unix_ms,
            scan_end_ms,
            scan_end_unix_ms,
            outcome.wardrive_entries,
            outcome.wardrive_extras,
        );
    }

    Ok(Some(BleCaptureOutcome {
        detection_alerts: outcome.detection_alerts,
        remote_id_alerts: outcome.remote_id_alerts,
        had_bulk,
        scan_end_ms,
        scan_end_unix_ms,
    }))
}

/// Run one BLE-role scan iteration.
///
/// Returns `Ok(true)` when the outer main loop should `continue` (all paths today).
pub(crate) fn run_ble_scan_tick(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    outbound: &mut OutboundBulkStore,
    ctx: &mut SnifferCtx<'_>,
    last_on_control_ms: &mut u64,
    schedule_sig_prev: &mut u64,
) -> Result<bool> {
    let control_ch = ctx.schedule.control_channel;
    let scan_only = ctx.cfg.scan_only();

    let Some(capture) = run_ble_capture_phase(wifi, ctx, rx_rx, outbound, control_ch, *seq)? else {
        return Ok(true);
    };

    let had_detection_alerts = !capture.detection_alerts.is_empty();
    let had_remote_id_alerts = !capture.remote_id_alerts.is_empty();
    let alerts = if had_detection_alerts {
        SweepAlerts::Ble {
            alerts: capture.detection_alerts,
            scan_end_ms: capture.scan_end_ms,
            scan_end_unix_ms: capture.scan_end_unix_ms,
        }
    } else if had_remote_id_alerts {
        SweepAlerts::RemoteId(capture.remote_id_alerts)
    } else {
        SweepAlerts::None
    };

    let return_label = if had_detection_alerts {
        "ble detection alert: return to control"
    } else if had_remote_id_alerts {
        "remote id alert: return to control"
    } else {
        "ble dump: return to control"
    };

    finish_sweep_on_control(
        wifi,
        ctx,
        rx_rx,
        control_ch,
        outbound,
        capture.had_bulk,
        scan_only,
        alerts,
        None,
        last_on_control_ms,
        schedule_sig_prev,
        seq,
        scan_batch_id,
        BulkDumpPlan::Ble,
        return_label,
    )?;

    Ok(true)
}
