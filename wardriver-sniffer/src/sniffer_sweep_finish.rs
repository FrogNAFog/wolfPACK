//! Shared post-capture sweep lifecycle: return to control, detection alerts, wolfPACK dump, cmd window.

use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::warn;
use wardriver_shared::spiram_queue::Receiver as EspnowRxReceiver;

use wardriver_core::lpr_camera_ble::BleCameraAlert;
use wardriver_core::sweep_finish_policy::{
    ble_detection_alert_v1_from, hop_resume_action, should_count_wardrive_suppressed_skip,
    HopResumeAction, HopResumeInput,
};
use wardriver_proto::{DetectionAlertV1, PeerJoinReason, RemoteIdAlertV1};
use wardriver_shared::espnow;
use wardriver_stats as stats;

use crate::sniffer_dispatch;
use crate::sniffer_espnow::{
    espnow_send_ble_entries, espnow_send_detection_alert, espnow_send_detection_ble_alert,
    espnow_send_probe_groups, espnow_send_remote_id_alert, espnow_send_wifi_batches,
    flush_sniffer_ack, SnifferCtx,
};
use crate::sniffer_outbound_bulk::{BulkDumpPlan, OutboundBulkStore};
use crate::sniffer_sweep::schedule_sig;
use crate::sniffer_wifi::{sniffer_boot_id, try_set_channel_with_recovery};

/// WiFi-only hop rotation state after a sweep finishes on the control channel.
pub(crate) struct SweepHopResume<'a> {
    pub next_channel_idx: &'a mut usize,
    pub aborted_due_to_pressure: bool,
    pub ended_max_off_break: bool,
}

/// Detection alerts raised during phase-A capture (media-specific wire encoding).
pub(crate) enum SweepAlerts {
    None,
    Wifi(Vec<DetectionAlertV1>),
    Ble {
        alerts: Vec<BleCameraAlert>,
        scan_end_ms: u32,
        scan_end_unix_ms: u64,
    },
    RemoteId(Vec<RemoteIdAlertV1>),
}

impl SweepAlerts {
    fn has_alerts(&self) -> bool {
        match self {
            Self::None => false,
            Self::Wifi(v) => !v.is_empty(),
            Self::Ble { alerts, .. } => !alerts.is_empty(),
            Self::RemoteId(v) => !v.is_empty(),
        }
    }

    fn return_label(&self) -> &'static str {
        match self {
            Self::Ble { .. } => "ble detection alert: return to control",
            Self::RemoteId(_) => "remote id alert: return to control",
            _ => "detection alert: return to control",
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct BulkDumpOutcome {
    pub dump_cycle_active: bool,
    pub dump_delivered: bool,
}

fn flush_ack_after_cmd_window(ctx: &mut SnifferCtx<'_>) {
    if sniffer_dispatch::post_dump_done_seen() {
        let _ = flush_sniffer_ack(ctx, true);
    } else {
        let _ = flush_sniffer_ack(ctx, false);
    }
}

fn apply_hop_resume(hr: &mut SweepHopResume<'_>) {
    match hop_resume_action(HopResumeInput {
        aborted_due_to_pressure: hr.aborted_due_to_pressure,
        ended_max_off_break: hr.ended_max_off_break,
        current_channel_idx: *hr.next_channel_idx,
    }) {
        HopResumeAction::KeepIndex => {}
        HopResumeAction::ClearAbortPressureFlag => {
            stats::SNIFFER_LAST_ABORT_PRESSURE.store(0, Ordering::Relaxed);
        }
        HopResumeAction::ResetIndexToZero => {
            *hr.next_channel_idx = 0;
            stats::SNIFFER_LAST_ABORT_PRESSURE.store(0, Ordering::Relaxed);
        }
    }
}

fn send_sweep_alerts(
    ctx: &mut SnifferCtx<'_>,
    control_ch: u8,
    alerts: &SweepAlerts,
    seq: &mut u32,
) {
    if !*ctx.control_established {
        return;
    }
    let Some(mac) = *ctx.control_mac else {
        return;
    };
    match alerts {
        SweepAlerts::Wifi(alerts) => {
            for alert in alerts {
                if let Err(e) = espnow_send_detection_alert(mac, *alert) {
                    warn!("sniffer: espnow_send_detection_alert: {e:#}");
                } else {
                    *seq = seq.wrapping_add(1);
                }
            }
        }
        SweepAlerts::Ble {
            alerts,
            scan_end_ms,
            scan_end_unix_ms,
        } => {
            for alert in alerts {
                let wire = ble_detection_alert_v1_from(
                    alert,
                    ctx.sniffer_id,
                    *seq,
                    sniffer_boot_id(),
                    *scan_end_ms,
                    *scan_end_unix_ms,
                );
                if espnow_send_detection_ble_alert(mac, wire).is_ok() {
                    *seq = seq.wrapping_add(1);
                } else {
                    warn!("sniffer: espnow_send_detection_ble_alert failed");
                }
            }
        }
        SweepAlerts::RemoteId(alerts) => {
            for alert in alerts {
                if let Err(e) = espnow_send_remote_id_alert(mac, *alert) {
                    warn!("sniffer: espnow_send_remote_id_alert: {e:#}");
                } else {
                    *seq = seq.wrapping_add(1);
                }
            }
        }
        SweepAlerts::None => {}
    }
    let _ = espnow::set_channel(control_ch);
}

/// `PeerJoin` + encrypted bulk dump when outbound has sendable frags.
pub(crate) fn try_bulk_dump_after_join<'a>(
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    control_ch: u8,
    outbound: &mut OutboundBulkStore,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    plan: BulkDumpPlan,
) -> BulkDumpOutcome {
    let mut outcome = BulkDumpOutcome::default();
    if !*ctx.control_established {
        return outcome;
    }
    let Some(mac) = *ctx.control_mac else {
        return outcome;
    };
    let sniffer_id = ctx.sniffer_id;

    match plan {
        BulkDumpPlan::Wifi => {
            let (wifi_batches, mut probe_groups_send) = outbound.build_wifi_send_view();
            let probe_frags = crate::sniffer_espnow::count_probe_send_frags(&probe_groups_send);
            let wifi_frags =
                crate::sniffer_espnow::count_wifi_send_frags_with_extras(&wifi_batches);
            let total_frags = probe_frags.saturating_add(wifi_frags);
            outcome.dump_cycle_active = true;
            let bulk_pressure = outbound.is_bulk_pressure();
            let age_pressure = outbound.is_age_pressure();
            sniffer_dispatch::trace_return_control_for_dump(
                sniffer_id,
                "wifi",
                control_ch,
                total_frags,
                bulk_pressure,
                age_pressure,
            );
            let join_ok = sniffer_dispatch::join_before_dump(
                sniffer_id,
                sniffer_boot_id(),
                PeerJoinReason::DumpWifi,
                "wifi",
                outbound.is_join_pressure(),
                || {
                    let _ = ctx.drain_rx(rx_rx);
                },
            );
            if !join_ok {
                warn!("sniffer: peer_join denied or timeout (wifi dump); skipping encrypted send");
                return outcome;
            }
            sniffer_dispatch::trace_begin_dump(sniffer_id, "wifi", total_frags);
            outcome.dump_delivered =
                sniffer_dispatch::dispatch_wifi_dump(probe_frags, wifi_frags, |burst| {
                    espnow_send_probe_groups(
                        mac,
                        sniffer_id,
                        seq,
                        scan_batch_id,
                        &mut probe_groups_send,
                        burst,
                    )?;
                    let _ = espnow::set_channel(control_ch)?;
                    espnow_send_wifi_batches(
                        mac,
                        sniffer_id,
                        seq,
                        scan_batch_id,
                        wifi_batches.clone(),
                        burst,
                    )?;
                    Ok(true)
                });
        }
        BulkDumpPlan::Ble => {
            let (ble_scan_end_ms, ble_scan_end_unix_ms, ble_entries, ble_extras) =
                outbound.build_ble_send_view();
            let ble_frags =
                crate::sniffer_espnow::count_ble_send_frags_with_extras(&ble_entries, &ble_extras);
            outcome.dump_cycle_active = true;
            let bulk_pressure = outbound.is_bulk_pressure();
            let age_pressure = outbound.is_age_pressure();
            sniffer_dispatch::trace_return_control_for_dump(
                sniffer_id,
                "ble",
                control_ch,
                ble_frags,
                bulk_pressure,
                age_pressure,
            );
            let join_ok = sniffer_dispatch::join_before_dump(
                sniffer_id,
                sniffer_boot_id(),
                PeerJoinReason::DumpBle,
                "ble",
                outbound.is_join_pressure(),
                || {
                    let _ = ctx.drain_rx(rx_rx);
                },
            );
            if !join_ok {
                warn!("sniffer: peer_join denied or timeout (ble dump); skipping encrypted send");
                return outcome;
            }
            sniffer_dispatch::trace_begin_dump(sniffer_id, "ble", ble_frags);
            outcome.dump_delivered = sniffer_dispatch::dispatch_ble_dump(ble_frags, |burst| {
                espnow_send_ble_entries(
                    mac,
                    sniffer_id,
                    seq,
                    scan_batch_id,
                    ble_scan_end_ms,
                    ble_scan_end_unix_ms,
                    &ble_entries,
                    &ble_extras,
                    burst,
                )?;
                Ok(true)
            });
        }
    }

    if outcome.dump_delivered {
        outbound.clear_on_delivered();
    }
    outcome
}

fn dump_media_label(plan: BulkDumpPlan) -> &'static str {
    match plan {
        BulkDumpPlan::Wifi => "wifi",
        BulkDumpPlan::Ble => "ble",
    }
}

/// Post-capture finish on the control channel (WiFi source-of-truth behavior).
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_sweep_on_control<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    control_ch: u8,
    outbound: &mut OutboundBulkStore,
    had_bulk: bool,
    scan_only: bool,
    alerts: SweepAlerts,
    mut hop_resume: Option<&mut SweepHopResume<'a>>,
    last_on_control_ms: &mut u64,
    schedule_sig_prev: &mut u64,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    dump_plan: BulkDumpPlan,
    return_label: &'static str,
) -> Result<()> {
    let has_alerts = alerts.has_alerts();
    let channel_label = if has_alerts {
        alerts.return_label()
    } else {
        return_label
    };

    if !try_set_channel_with_recovery(wifi, control_ch, channel_label) {
        warn!("sniffer: failed to return to control channel {control_ch} ({channel_label})");
        std::thread::sleep(Duration::from_millis(50));
        sniffer_dispatch::record_bulk_cycle(had_bulk, scan_only, false);
        return Ok(());
    }
    *last_on_control_ms = stats::now_ms();

    let mut refresh_cycle_active = false;
    let mut refresh_delivered = false;
    let mut dump_cycle_active = false;
    let mut dump_delivered = false;

    if has_alerts {
        send_sweep_alerts(ctx, control_ch, &alerts, seq);
    }

    if should_count_wardrive_suppressed_skip(scan_only, has_alerts) {
        stats::SNIFFER_WARDRIVE_SUPPRESSED_SKIPS.fetch_add(1, Ordering::Relaxed);
    }

    let frags = outbound.pending_send_frags(dump_plan);
    if !scan_only && frags > 0 && *ctx.control_established {
        let dump = try_bulk_dump_after_join(
            ctx,
            rx_rx,
            control_ch,
            outbound,
            seq,
            scan_batch_id,
            dump_plan,
        );
        dump_cycle_active = dump.dump_cycle_active;
        dump_delivered = dump.dump_delivered;
        sniffer_dispatch::record_bulk_cycle(had_bulk, scan_only, dump_delivered);
    } else if *ctx.control_established && ctx.control_mac.is_some() {
        refresh_cycle_active = true;
        refresh_delivered =
            sniffer_dispatch::run_peer_refresh_join(ctx.sniffer_id, control_ch, || {
                let _ = ctx.drain_rx(rx_rx);
            });
        let _ = flush_sniffer_ack(ctx, refresh_delivered);
        sniffer_dispatch::record_bulk_cycle(had_bulk, scan_only, refresh_delivered);
    } else {
        sniffer_dispatch::record_bulk_cycle(had_bulk, scan_only, false);
    }

    if let Some(hr) = hop_resume.as_mut() {
        apply_hop_resume(hr);
    }

    ctx.flush_pending_time_set();

    let state_before = *ctx.state;
    let sig_before = schedule_sig(ctx.schedule);
    let window_deadline = stats::now_ms().saturating_add(sniffer_dispatch::CONTROL_CMD_WINDOW_MS);
    sniffer_dispatch::drain_control_cmd_window(window_deadline, || {
        ctx.drain_rx(rx_rx)?;
        let sig_after = schedule_sig(ctx.schedule);
        Ok(*ctx.state != state_before || sig_after != sig_before)
    })?;
    let sig_after = schedule_sig(ctx.schedule);
    if *ctx.state != state_before || sig_after != sig_before {
        *schedule_sig_prev = sig_after;
        if let Some(hr) = hop_resume {
            if sig_after != sig_before {
                *hr.next_channel_idx = 0;
            }
        }
    }
    flush_ack_after_cmd_window(ctx);

    let media = dump_media_label(dump_plan);
    if dump_cycle_active {
        sniffer_dispatch::trace_end_dump(
            ctx.sniffer_id,
            media,
            dump_delivered,
            sniffer_dispatch::post_dump_done_seen(),
        );
    } else if refresh_cycle_active {
        sniffer_dispatch::trace_end_dump(ctx.sniffer_id, "refresh", refresh_delivered, false);
    }

    Ok(())
}
