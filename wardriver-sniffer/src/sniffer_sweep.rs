//! WiFi channel schedule, sweep hop capture, and bulk transport helpers.

use std::sync::atomic::Ordering;
use wardriver_shared::spiram_queue::Receiver as PromiscReceiver;

use wardriver_shared::spiram_queue::Receiver as EspnowRxReceiver;

use anyhow::Result;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::warn;

use crate::sniffer_espnow::{espnow_send_ssid_watch_alert, sniffer_unix_now_ms, SnifferCtx};
use crate::sniffer_outbound_bulk::BulkDumpPlan;
use crate::sniffer_outbound_bulk::OutboundBulkStore;
use crate::sniffer_sweep_finish::{finish_sweep_on_control, SweepAlerts, SweepHopResume};
use crate::sniffer_wifi::sniffer_boot_id;
use crate::sniffer_wifi::try_set_channel_with_recovery;
use crate::wifi_promisc::{self, ProbeReqBurstLocal, PromiscRecord};
use wardriver_core::channel_plan::is_dfs_channel;
use wardriver_core::sniffer_schedule::{self, SnifferScheduleSig, SNIFFER_SWEEP_MAX_DWELL_MS};
use wardriver_core::sniffer_sweep::{
    self, had_wifi_bulk, throttle_wifi_entries_with_extras, ProbeReqBurstInput,
    BSSID_SEND_MIN_INTERVAL_MS,
};
use wardriver_proto::{
    self as proto, ApEntryV1, ControlCmdV1, DetectionAlertV1, ProbeReqBurstV1, RemoteIdAlertV1,
    ScanEntryExtras, SnifferState,
};
use wardriver_shared::espnow;
use wardriver_stats as stats;

/// Hop cadence guardrail: during sweeping we want ≥ 2 hops/sec instantaneous.
pub(crate) const MAX_DWELL_MS: u16 = SNIFFER_SWEEP_MAX_DWELL_MS;

/// Early abort pressure hysteresis: require this many consecutive channels with promisc drops.
pub(crate) const PRESSURE_HYSTERESIS_CONSEC: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct ChannelSchedule {
    pub control_channel: u8,
    pub dwell_ms: u16,
    pub chan_count: u8,
    pub channels: [u8; wardriver_proto::CHANNELS_MAX],
}

impl Default for ChannelSchedule {
    fn default() -> Self {
        let mut channels = [0u8; wardriver_proto::CHANNELS_MAX];
        channels[0] = 1;
        channels[1] = 6;
        channels[2] = 11;
        Self {
            control_channel: proto::CONTROL_AP_CHANNEL,
            dwell_ms: wardriver_proto::DEFAULT_SCAN_DWELL_MS,
            chan_count: 3,
            channels,
        }
    }
}

impl ChannelSchedule {
    pub fn from_cmd(cmd: ControlCmdV1) -> Self {
        let sig = SnifferScheduleSig::from_control_cmd(&cmd);
        Self {
            control_channel: sig.control_channel,
            dwell_ms: sig.dwell_ms,
            chan_count: sig.chan_count,
            channels: sig.channels,
        }
    }

    pub fn scan_channels_iter(&self) -> impl Iterator<Item = u8> + '_ {
        self.channels
            .iter()
            .take(self.chan_count as usize)
            .copied()
            .filter(|c| *c != 0)
    }

    /// Drop WiFi hop list; keep control channel and dwell for wire compat (BLE role).
    pub fn clear_wifi_hop_channels(&mut self) {
        self.chan_count = 0;
        self.channels = [0u8; wardriver_proto::CHANNELS_MAX];
    }
}

fn schedule_to_sig(s: &ChannelSchedule) -> SnifferScheduleSig {
    SnifferScheduleSig {
        control_channel: s.control_channel,
        dwell_ms: s.dwell_ms,
        chan_count: s.chan_count,
        channels: s.channels,
    }
}

pub(crate) fn schedule_sig(s: &ChannelSchedule) -> u64 {
    sniffer_schedule::schedule_sig(&schedule_to_sig(s))
}

pub(crate) use sniffer_sweep::{filter_probe_groups_by_alerted_srcs, merge_probe_bursts_by_src};

/// Convert local promisc probe summaries to wire rows (capped per channel).
pub(crate) fn local_probe_bursts_to_wire(
    probe_bursts: Vec<ProbeReqBurstLocal>,
) -> Vec<ProbeReqBurstV1> {
    sniffer_sweep::local_probe_bursts_to_wire(
        probe_bursts
            .into_iter()
            .map(|b| ProbeReqBurstInput {
                src: b.src,
                wildcard_count: b.wildcard_count,
                channel: b.channel,
                first_seen_ms32: b.first_seen_ms32,
                last_seen_ms32: b.last_seen_ms32,
                rssi_min: b.rssi_min,
                rssi_max: b.rssi_max,
                rssi_avg_q8: b.rssi_avg_q8,
                primary_sig_matched: b.primary_sig_matched,
            })
            .collect(),
    )
}

/// Update pressure hysteresis from promisc queue drop delta; returns (new_consec, should_abort).
pub(crate) fn update_pressure_hysteresis(
    promisc_drops_before: u32,
    pressure_consec: u8,
) -> (u8, bool) {
    let drops_now =
        wardriver_stats::PROMISC_RX_DROPPED_QUEUE_FULL.load(std::sync::atomic::Ordering::Relaxed);
    let delta = drops_now.saturating_sub(promisc_drops_before);
    let new_consec = if delta > 0 {
        pressure_consec.saturating_add(1)
    } else {
        0
    };
    let abort = new_consec >= PRESSURE_HYSTERESIS_CONSEC;
    if abort {
        wardriver_stats::SNIFFER_LAST_ABORT_PRESSURE.store(1, std::sync::atomic::Ordering::Relaxed);
    }
    (new_consec, abort)
}

/// RF capture phase: promisc channel hops, per-channel dwell, pressure abort.
pub(crate) struct WifiSweepHopOutcome {
    pub groups: Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)>,
    pub probe_groups: Vec<(u32, u64, Vec<ProbeReqBurstV1>)>,
    pub detection_alerts: Vec<DetectionAlertV1>,
    pub remote_id_alerts: Vec<RemoteIdAlertV1>,
    pub aborted_due_to_pressure: bool,
    pub ended_max_off_break: bool,
}

/// Phase A: scan hard (RF capture). Keeps promisc enabled across the whole sweep/session.
///
/// Returns `Ok(None)` when the schedule has no channels (caller should yield briefly).
pub(crate) fn run_wifi_sweep_hop<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    promisc_rx: &PromiscReceiver<PromiscRecord>,
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    last_on_control_ms: u64,
    next_channel_idx: &mut usize,
    schedule_sig_prev: &mut u64,
    alert_seq: u32,
) -> Result<Option<WifiSweepHopOutcome>> {
    let sig_now = schedule_sig(ctx.schedule);
    if *schedule_sig_prev != sig_now {
        *schedule_sig_prev = sig_now;
        *next_channel_idx = 0;
    }

    // Descending channel-number order for LPR-camera rendezvous.
    let channels = wardriver_core::channel_plan::wardrive_hop_channels(
        &ctx.schedule.scan_channels_iter().collect::<Vec<_>>(),
    );
    if channels.is_empty() {
        return Ok(None);
    }

    // add control channel to allow checking if we want to drain espnow rx
    let control_ch = ctx.schedule.control_channel;
    let n_ch = channels.len();
    let dwell_ms = ctx.schedule.dwell_ms.min(MAX_DWELL_MS);

    let mut all_entries: Vec<ApEntryV1> = Vec::new();
    let mut all_extras: Vec<ScanEntryExtras> = Vec::new();
    let mut probe_groups: Vec<(u32, u64, Vec<ProbeReqBurstV1>)> =
        Vec::with_capacity(channels.len());
    let mut aborted_due_to_pressure = false;
    let mut ended_max_off_break = false;
    let mut pressure_consec: u8 = 0;

    let promisc_drops_before = stats::PROMISC_RX_DROPPED_QUEUE_FULL.load(Ordering::Relaxed);
    let scan_only = ctx.cfg.scan_only();
    let mut cap = wifi_promisc::PromiscCapture::begin(
        wifi.wifi_mut(),
        promisc_rx,
        ctx.sniffer_id,
        ctx.cfg.detection_disable_wifi_mask,
        ctx.cfg.remote_id_disable_wifi_mask,
        alert_seq,
        sniffer_boot_id(),
        scan_only,
    )?;
    let start_idx = if n_ch == 0 {
        0
    } else {
        *next_channel_idx % n_ch
    };
    let mut detection_alerts: Vec<DetectionAlertV1> = Vec::new();
    let mut remote_id_alerts: Vec<RemoteIdAlertV1> = Vec::new();

    let max_off_ms = wardriver_core::dwell::DwellAndSchedule::max_off_control_ms();
    let mut scan_end_ms_max: u32 = 0;
    let mut anchor_unix: u64 = 0;

    // Phase A hops resume from `next_channel_idx` and run until the off-control budget is
    // exhausted (multi-rotation wardrive) or alert/pressure/idle breaks. `ended_max_off_break`
    // preserves `next_channel_idx` across dumps so the schedule rotates instead of restarting.
    let mut step = 0usize;

    while *ctx.state == SnifferState::Scanning {
        let off_control_ms = stats::now_ms().saturating_sub(last_on_control_ms);
        if step > 0 && off_control_ms >= max_off_ms {
            let i = (start_idx + step) % n_ch;
            *next_channel_idx = i;
            ended_max_off_break = true;
            break;
        }

        let i = (start_idx + step) % n_ch;
        step = step.saturating_add(1);
        let ch = channels[i];

        let t_set0 = std::time::Instant::now();
        // DFS and non-DFS hops share this path (no passive-scan preamble).
        if !try_set_channel_with_recovery(wifi, ch, "sweep") {
            stats::increment_dfs_hop_skip();
            warn!(
                "sniffer: skip scan channel {ch} (DFS={})",
                is_dfs_channel(ch),
            );
            continue;
        }
        let set_us = t_set0.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;

        let (entries, extras, probe_bursts) = cap.capture_channel(
            ch,
            dwell_ms,
            ctx.bssid_denylist,
            ctx.ssid_denylist,
            ctx.ssid_watch,
        );

        // SSID watch alerts do not abort the sweep; send when tuned to the control channel.
        if ch == control_ch {
            let watch_alerts = cap.take_ssid_watch_alerts();
            if !watch_alerts.is_empty() && *ctx.control_established {
                if let Some(mac) = *ctx.control_mac {
                    for alert in watch_alerts {
                        if let Err(e) = espnow_send_ssid_watch_alert(mac, &alert) {
                            warn!("sniffer: espnow_send_ssid_watch_alert: {e:#}");
                        }
                    }
                }
            }
        }

        if !scan_only {
            let scan_end_ms = stats::now_ms() as u32;
            if !entries.is_empty() {
                if scan_end_ms >= scan_end_ms_max {
                    scan_end_ms_max = scan_end_ms;
                    anchor_unix = sniffer_unix_now_ms();
                }
                // Per-dwell rows; RSSI-aware 500 ms throttle runs at sweep end.
                all_entries.extend(entries);
                all_extras.extend(extras);
            }
            let out = local_probe_bursts_to_wire(probe_bursts);
            if !out.is_empty() {
                probe_groups.push((scan_end_ms, sniffer_unix_now_ms(), out));
            }
        }

        let channel_alerts = cap.take_alerts();
        if !channel_alerts.is_empty() {
            detection_alerts.extend(channel_alerts);
            *next_channel_idx = (i + 1) % n_ch;
            break;
        }
        let channel_rid_alerts = cap.take_remote_id_alerts();
        if !channel_rid_alerts.is_empty() {
            remote_id_alerts.extend(channel_rid_alerts);
            *next_channel_idx = (i + 1) % n_ch;
            break;
        }

        let (new_pressure_consec, abort_pressure) =
            update_pressure_hysteresis(promisc_drops_before, pressure_consec);
        pressure_consec = new_pressure_consec;
        if abort_pressure {
            aborted_due_to_pressure = true;
            *next_channel_idx = (i + 1) % n_ch;
            break;
        }

        // Rolling average feeds DwellAndSchedule control-silence timeout on the control node.
        stats::sniffer_record_hop_timing(set_us);
        wardriver_shared::task_wdt::reset_if_subscribed();

        // ESP-NOW is channel-bound; only drain when we were tuned to the control channel.

        if ch == control_ch {
            ctx.drain_rx(rx_rx)?;
        }
    }

    probe_groups = merge_probe_bursts_by_src(probe_groups);
    cap.extend_alerted_probe_srcs(ctx.alerted_probe_srcs);

    let (all_entries, all_extras) =
        throttle_wifi_entries_with_extras(all_entries, all_extras, BSSID_SEND_MIN_INTERVAL_MS);

    let groups = if all_entries.is_empty() {
        Vec::new()
    } else {
        vec![(scan_end_ms_max, anchor_unix, all_entries, all_extras)]
    };

    cap.end(wifi.wifi_mut())?;

    Ok(Some(WifiSweepHopOutcome {
        groups,
        probe_groups,
        detection_alerts,
        remote_id_alerts,
        aborted_due_to_pressure,
        ended_max_off_break,
    }))
}

/// Remote ID alert finish: broadcast alerts, optional wardrive dump on same visit, cmd window.
pub(crate) fn run_wifi_sweep_remote_id_finish<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    control_ch: u8,
    alerts: Vec<RemoteIdAlertV1>,
    outbound: &mut OutboundBulkStore,
    had_bulk: bool,
    scan_only: bool,
    aborted_due_to_pressure: bool,
    ended_max_off_break: bool,
    last_on_control_ms: &mut u64,
    next_channel_idx: &'a mut usize,
    schedule_sig_prev: &mut u64,
    seq: &mut u32,
    scan_batch_id: &mut u32,
) -> Result<()> {
    if alerts.is_empty() {
        return Ok(());
    }
    finish_sweep_on_control(
        wifi,
        ctx,
        rx_rx,
        control_ch,
        outbound,
        had_bulk,
        scan_only,
        SweepAlerts::RemoteId(alerts),
        Some(&mut SweepHopResume {
            next_channel_idx,
            aborted_due_to_pressure,
            ended_max_off_break,
        }),
        last_on_control_ms,
        schedule_sig_prev,
        seq,
        scan_batch_id,
        BulkDumpPlan::Wifi,
        "remote id alert: return to control",
    )
}

/// Detection alert finish: broadcast alerts, optional wardrive dump on same visit, cmd window.
pub(crate) fn run_wifi_sweep_alert_finish<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    control_ch: u8,
    alerts: Vec<DetectionAlertV1>,
    outbound: &mut OutboundBulkStore,
    had_bulk: bool,
    scan_only: bool,
    aborted_due_to_pressure: bool,
    ended_max_off_break: bool,
    last_on_control_ms: &mut u64,
    next_channel_idx: &'a mut usize,
    schedule_sig_prev: &mut u64,
    seq: &mut u32,
    scan_batch_id: &mut u32,
) -> Result<()> {
    if alerts.is_empty() {
        return Ok(());
    }
    finish_sweep_on_control(
        wifi,
        ctx,
        rx_rx,
        control_ch,
        outbound,
        had_bulk,
        scan_only,
        SweepAlerts::Wifi(alerts),
        Some(&mut SweepHopResume {
            next_channel_idx,
            aborted_due_to_pressure,
            ended_max_off_break,
        }),
        last_on_control_ms,
        schedule_sig_prev,
        seq,
        scan_batch_id,
        BulkDumpPlan::Wifi,
        "detection alert: return to control",
    )
}

pub(crate) fn run_wifi_sweep_finish<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ctx: &mut SnifferCtx<'a>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    control_ch: u8,
    hop: WifiSweepHopOutcome,
    outbound: &mut OutboundBulkStore,
    last_on_control_ms: &mut u64,
    next_channel_idx: &'a mut usize,
    schedule_sig_prev: &mut u64,
    seq: &mut u32,
    scan_batch_id: &mut u32,
) -> Result<()> {
    let WifiSweepHopOutcome {
        groups,
        mut probe_groups,
        detection_alerts,
        remote_id_alerts,
        aborted_due_to_pressure,
        ended_max_off_break,
    } = hop;

    probe_groups = filter_probe_groups_by_alerted_srcs(probe_groups, ctx.alerted_probe_srcs);

    let now_unix = sniffer_unix_now_ms();
    let had_bulk = had_wifi_bulk(&groups, &probe_groups);
    let scan_only = ctx.cfg.scan_only();
    if !scan_only {
        outbound.insert_wifi(now_unix, groups);
        outbound.insert_probes(now_unix, probe_groups);
    }

    if !detection_alerts.is_empty() {
        return run_wifi_sweep_alert_finish(
            wifi,
            ctx,
            rx_rx,
            control_ch,
            detection_alerts,
            outbound,
            had_bulk,
            scan_only,
            aborted_due_to_pressure,
            ended_max_off_break,
            last_on_control_ms,
            next_channel_idx,
            schedule_sig_prev,
            seq,
            scan_batch_id,
        );
    }

    if !remote_id_alerts.is_empty() {
        return run_wifi_sweep_remote_id_finish(
            wifi,
            ctx,
            rx_rx,
            control_ch,
            remote_id_alerts,
            outbound,
            had_bulk,
            scan_only,
            aborted_due_to_pressure,
            ended_max_off_break,
            last_on_control_ms,
            next_channel_idx,
            schedule_sig_prev,
            seq,
            scan_batch_id,
        );
    }

    finish_sweep_on_control(
        wifi,
        ctx,
        rx_rx,
        control_ch,
        outbound,
        had_bulk,
        scan_only,
        SweepAlerts::None,
        Some(&mut SweepHopResume {
            next_channel_idx,
            aborted_due_to_pressure,
            ended_max_off_break,
        }),
        last_on_control_ms,
        schedule_sig_prev,
        seq,
        scan_batch_id,
        BulkDumpPlan::Wifi,
        "wifi dump: return to control",
    )
}
