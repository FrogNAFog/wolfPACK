//! ESP-NOW RX dispatch and control-command handling for the sniffer role.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use wardriver_core::ssid_watch::SsidWatch;
use wardriver_shared::spiram_queue::Receiver as EspnowRxReceiver;

use anyhow::{anyhow, Result};
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_sys as sys;
use log::{info, warn};

use crate::sniffer_config::SnifferConfig;
use crate::sniffer_dispatch::TransportBurst;
use crate::sniffer_sweep::{self, ChannelSchedule};
use crate::sniffer_wifi::{sniffer_boot_id, sniffer_five_ghz_capable};
use wardriver_proto::{
    self as proto, ApEntryV1, BleEntryV1, BleScanFragV1Header, ControlCmd, ControlCmdV1,
    DetectionAlertV1, DetectionBleAlertV1, FrameType, ProbeReqBurstV1, ProbeReqFragV1Header,
    RemoteIdAlertV1, ScanEntryExtras, ScanFragV1Header, SnifferAckV1, SnifferRole, SnifferState,
};
use wardriver_shared::espnow;

/// After the first few `SnifferAck`s, also send a **broadcast** copy so the control node
/// receives the frame before it has `esp_now_add_peer` for this sniffer (IDF drops encrypted
/// unicast to unknown peers). Steady state stays unicast-only once this counter reaches 0.
const SNIFFER_ACK_BOOTSTRAP_BROADCAST_INITIAL: u32 = 64;
static SNIFFER_ACK_BOOTSTRAP_REMAINING: AtomicU32 =
    AtomicU32::new(SNIFFER_ACK_BOOTSTRAP_BROADCAST_INITIAL);

/// Set after `ReleaseBinding`; main loop calls [`crate::sniffer_link::SnifferLinkState::reset_fast_join`].
pub(crate) static FLEET_FAST_JOIN_RESET: AtomicBool = AtomicBool::new(false);

pub(crate) fn sniffer_unix_now_ms() -> u64 {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, core::ptr::null_mut());
    }
    if tv.tv_sec <= 0 {
        return 0;
    }
    let ms = (tv.tv_sec as i64)
        .saturating_mul(1000)
        .saturating_add((tv.tv_usec as i64) / 1000);
    if ms <= 0 {
        return 0;
    }
    ms as u64
}

pub(crate) fn send_ack(
    control_mac: [u8; 6],
    sniffer_id: u32,
    ack_cmd_seq: u32,
    state: SnifferState,
    cfg: &SnifferConfig,
) -> Result<()> {
    let role = SnifferRole::from_wire(cfg.role);
    let ble_capable: u8 = if cfg!(feature = "ble") { 1 } else { 0 };
    let payload = proto::pack_sniffer_ack(SnifferAckV1 {
        sniffer_id,
        ack_cmd_seq,
        state,
        role,
        ble_capable,
        sniffer_boot_id: sniffer_boot_id(),
        free_heap_bytes: wardriver_stats::free_heap_internal_plus_spiram_bytes(),
        total_heap_bytes: wardriver_stats::total_heap_internal_plus_spiram_bytes(),
        cpu_load_pct: wardriver_shared::cpu_load::pct(),
        five_ghz_capable: sniffer_five_ghz_capable(),
        // PROTO cleanup candidate: mirrors five_ghz_capable on the wire until next PROTO bump.
        dfs_capable: sniffer_five_ghz_capable(),
        bulk_lost_cycles: wardriver_stats::sniffer_bulk_lost_cycles(),
    });
    let r = espnow::send(control_mac, &payload);
    loop {
        let c = SNIFFER_ACK_BOOTSTRAP_REMAINING.load(Ordering::Relaxed);
        if c == 0 {
            break;
        }
        if SNIFFER_ACK_BOOTSTRAP_REMAINING
            .compare_exchange_weak(c, c - 1, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            let _ = espnow::send(espnow::BROADCAST_MAC, &payload);
            break;
        }
    }
    r
}

/// Send `SnifferAck` when `pending_ack` or `force` (post-dump heartbeat).
///
/// `force` covers the post-HTTP/ESP-NOW path: control must see fresh `last_seen` + `state`
/// even when HTTP blocked RX and no Ping set `pending_ack`.
pub(crate) fn flush_sniffer_ack(ctx: &mut SnifferCtx<'_>, force: bool) -> Result<()> {
    if !*ctx.control_established {
        return Ok(());
    }
    let Some(mac) = *ctx.control_mac else {
        return Ok(());
    };
    if !*ctx.pending_ack && !force {
        return Ok(());
    }
    match send_ack(mac, ctx.sniffer_id, *ctx.last_cmd_seq, *ctx.state, ctx.cfg) {
        Ok(()) => {
            *ctx.pending_ack = false;
            Ok(())
        }
        Err(e) => {
            wardriver_stats::SNIFFER_ACK_TX_FAILED.fetch_add(1, Ordering::Relaxed);
            warn!("sniffer: send_ack failed (non-fatal): {e:#}");
            Err(e)
        }
    }
}

pub(crate) fn sniffer_id_from_mac() -> Result<u32> {
    let mut mac = [0u8; 6];
    unsafe {
        let rc = esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_STA,
        );
        if rc != 0 {
            return Err(anyhow!("esp_read_mac rc={rc}"));
        }
    }
    let mut h: u32 = 2166136261;
    for b in mac {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    Ok(h)
}

const MAX_PROBE_BURSTS_PER_SWEEP: usize = proto::MAX_PROBE_REQ_PER_FRAG * 8; // conservative cap

/// Immediate LPR-camera WiFi alert (broadcast ESP-NOW; never HTTP).
pub(crate) fn espnow_send_detection_alert(
    _mac: [u8; 6],
    alert: DetectionAlertV1,
) -> anyhow::Result<()> {
    let payload = proto::pack_detection_alert(alert);
    espnow::send(espnow::BROADCAST_MAC, &payload)?;
    Ok(())
}

/// Immediate SSID watch beacon hit (broadcast ESP-NOW; no SD log on control).
pub(crate) fn espnow_send_ssid_watch_alert(
    _mac: [u8; 6],
    alert: &proto::SsidWatchAlertV1,
) -> anyhow::Result<()> {
    let payload = proto::pack_ssid_watch_alert(alert)?;
    espnow::send(espnow::BROADCAST_MAC, &payload)?;
    Ok(())
}

/// Immediate LPR-camera BLE / mobile-surveillance alert (broadcast ESP-NOW; never HTTP).
pub(crate) fn espnow_send_detection_ble_alert(
    _mac: [u8; 6],
    alert: DetectionBleAlertV1,
) -> anyhow::Result<()> {
    let payload = proto::pack_detection_ble_alert(alert);
    espnow::send(espnow::BROADCAST_MAC, &payload)?;
    Ok(())
}

/// Immediate Remote ID UAS alert (broadcast ESP-NOW; never HTTP).
pub(crate) fn espnow_send_remote_id_alert(
    _mac: [u8; 6],
    alert: RemoteIdAlertV1,
) -> anyhow::Result<()> {
    let payload = proto::pack_remote_id_alert(alert);
    espnow::send(espnow::BROADCAST_MAC, &payload)?;
    Ok(())
}

/// Max AP rows per ESP-NOW reassembly group (`ScanFragV1Header::frag_total` is `u8`).
pub(crate) use wardriver_core::sniffer_frag::{
    count_ble_send_frags_with_extras, count_probe_send_frags, count_wifi_send_frags_with_extras,
    wifi_reassembly_ranges,
};

fn espnow_send_wifi_ap_chunk(
    mac: [u8; 6],
    sniffer_id: u32,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
    aps: &[ApEntryV1],
    extras: &[ScanEntryExtras],
    burst: &mut TransportBurst,
) -> anyhow::Result<()> {
    if aps.is_empty() {
        return Ok(());
    }

    let chunks = proto::chunk_aps_for_scan_frag_ranges(aps, extras);

    let frag_total = chunks.len();
    if frag_total == 0 {
        return Ok(());
    }
    debug_assert!(frag_total <= u8::MAX as usize);
    let frag_total_u8 = frag_total as u8;

    for (frag_idx, (start, end)) in chunks.into_iter().enumerate() {
        let chunk = &aps[start..end];
        let mut chunk_ex = if extras.len() >= end {
            extras[start..end].to_vec()
        } else {
            vec![ScanEntryExtras::default(); chunk.len()]
        };
        chunk_ex.resize(chunk.len(), ScanEntryExtras::default());
        let hdr = ScanFragV1Header {
            sniffer_id,
            seq: *seq,
            scan_batch_id: *scan_batch_id,
            sniffer_boot_id: sniffer_boot_id(),
            sniffer_scan_end_ms: scan_end_ms,
            sniffer_scan_end_unix_ms: scan_end_unix_ms,
            frag_idx: frag_idx as u8,
            frag_total: frag_total_u8,
            ap_count: chunk.len() as u8,
            flags: burst.flags_for_next_send(),
        };
        let payload = proto::pack_scan_frag_with_extras(hdr, chunk, &chunk_ex)?;
        let _ = espnow::send(mac, &payload);
    }
    *seq = seq.wrapping_add(1);
    *scan_batch_id = scan_batch_id.wrapping_add(1);
    Ok(())
}

/// Send WiFi scan frags over ESP-NOW (consumes `wifi_batches`).
pub(crate) fn espnow_send_wifi_batches(
    mac: [u8; 6],
    sniffer_id: u32,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    wifi_batches: Vec<(u32, u64, Vec<ApEntryV1>, Vec<ScanEntryExtras>)>,
    burst: &mut TransportBurst,
) -> anyhow::Result<()> {
    for (scan_end_ms, scan_end_unix_ms, aps, extras) in wifi_batches {
        if aps.is_empty() {
            continue;
        }
        for (offset, end) in wifi_reassembly_ranges(&aps, &extras) {
            if offset > 0 {
                wardriver_stats::SNIFFER_TRANSPORT_BATCHES_SPLIT.fetch_add(1, Ordering::Relaxed);
            }
            espnow_send_wifi_ap_chunk(
                mac,
                sniffer_id,
                seq,
                scan_batch_id,
                scan_end_ms,
                scan_end_unix_ms,
                &aps[offset..end],
                if extras.len() >= end {
                    &extras[offset..end]
                } else {
                    &[]
                },
                burst,
            )?;
        }
    }
    Ok(())
}

/// Send probe burst frags over ESP-NOW (drains `probe_groups`).
pub(crate) fn espnow_send_probe_groups(
    mac: [u8; 6],
    sniffer_id: u32,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    probe_groups: &mut Vec<(u32, u64, Vec<ProbeReqBurstV1>)>,
    burst: &mut TransportBurst,
) -> anyhow::Result<()> {
    for (scan_end_ms, scan_end_unix_ms, mut bursts) in probe_groups.drain(..) {
        if bursts.len() > MAX_PROBE_BURSTS_PER_SWEEP {
            warn!(
                "sniffer: truncating probe burst list {} -> {}",
                bursts.len(),
                MAX_PROBE_BURSTS_PER_SWEEP
            );
            bursts.truncate(MAX_PROBE_BURSTS_PER_SWEEP);
        }
        let frag_total =
            (bursts.len() + proto::MAX_PROBE_REQ_PER_FRAG - 1) / proto::MAX_PROBE_REQ_PER_FRAG;
        if frag_total == 0 {
            continue;
        }
        debug_assert!(frag_total <= u8::MAX as usize);
        let frag_total_u8 = frag_total as u8;
        let mut idx = 0usize;
        for frag_idx in 0..frag_total {
            let end = (idx + proto::MAX_PROBE_REQ_PER_FRAG).min(bursts.len());
            let chunk = &bursts[idx..end];
            idx = end;
            let hdr = ProbeReqFragV1Header {
                sniffer_id,
                seq: *seq,
                scan_batch_id: *scan_batch_id,
                sniffer_boot_id: sniffer_boot_id(),
                sniffer_scan_end_ms: scan_end_ms,
                sniffer_scan_end_unix_ms: scan_end_unix_ms,
                frag_idx: frag_idx as u8,
                frag_total: frag_total_u8,
                entry_count: chunk.len() as u8,
                flags: burst.flags_for_next_send(),
            };
            let payload = proto::pack_probe_req_frag(hdr, chunk)?;
            let _ = espnow::send(mac, &payload);
        }
        *seq = seq.wrapping_add(1);
        *scan_batch_id = scan_batch_id.wrapping_add(1);
    }
    Ok(())
}

/// Send BLE sweep frags over ESP-NOW.
pub(crate) fn espnow_send_ble_entries(
    mac: [u8; 6],
    sniffer_id: u32,
    seq: &mut u32,
    scan_batch_id: &mut u32,
    scan_end_ms: u32,
    scan_end_unix_ms: u64,
    entries: &[BleEntryV1],
    extras: &[ScanEntryExtras],
    burst: &mut TransportBurst,
) -> anyhow::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let chunks = proto::chunk_ble_scan_frag_ranges(entries, extras);
    let frag_total = chunks.len();
    if frag_total == 0 || frag_total > u8::MAX as usize {
        return Ok(());
    }
    let frag_total_u8 = frag_total as u8;
    for (frag_idx, (start, end)) in chunks.into_iter().enumerate() {
        let chunk = &entries[start..end];
        let chunk_ex = if extras.len() >= end {
            extras[start..end].to_vec()
        } else {
            let mut padded = vec![ScanEntryExtras::default(); chunk.len()];
            if extras.len() > start {
                let n = extras.len() - start;
                padded[..n].clone_from_slice(&extras[start..]);
            }
            padded
        };
        let hdr = BleScanFragV1Header {
            sniffer_id,
            seq: *seq,
            scan_batch_id: *scan_batch_id,
            sniffer_boot_id: sniffer_boot_id(),
            sniffer_scan_end_ms: scan_end_ms,
            sniffer_scan_end_unix_ms: scan_end_unix_ms,
            frag_idx: frag_idx as u8,
            frag_total: frag_total_u8,
            entry_count: chunk.len() as u8,
            flags: burst.flags_for_next_send(),
        };
        let payload = proto::pack_ble_scan_frag_with_extras(hdr, chunk, &chunk_ex)?;
        let _ = espnow::send(mac, &payload);
    }
    *seq = seq.wrapping_add(1);
    *scan_batch_id = scan_batch_id.wrapping_add(1);
    Ok(())
}

/// Recon session parameters set from [`FrameType::ReconCmd`] (main loop owns promisc capture).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReconSessionParams {
    pub session_id: u32,
    pub end_mono_ms: u64,
    pub dwell_ms: u16,
    pub control_ch: u8,
}

pub(crate) static RECON_SESSION: Mutex<Option<ReconSessionParams>> = Mutex::new(None);

/// Sync the six control-offered policy fields into [`SnifferConfig`], saving to NVS only when a
/// value actually changed. `wardrive_suppressed` and `ble_active_scan` arrive on the wire as
/// `u8` flags (`0`/non-zero) and are stored as `bool`; they are logged when they change.
/// `wardrive_suppressed` is intentionally **not** persisted (scan-only is a per-session mode),
/// matching the previous inline behavior at the `ControlCmd` and `ControlWelcome` sites.
fn apply_policy_fields(
    cfg: &mut SnifferConfig,
    nvs: &EspNvsPartition<NvsDefault>,
    detection_disable_wifi_mask: u32,
    detection_disable_ble_mask: u32,
    remote_id_disable_mask: u32,
    wardrive_suppressed: u8,
    ble_active_scan: u8,
) {
    if cfg.detection_disable_wifi_mask != detection_disable_wifi_mask {
        cfg.detection_disable_wifi_mask = detection_disable_wifi_mask;
        let _ = cfg.save(nvs);
    }
    if cfg.detection_disable_ble_mask != detection_disable_ble_mask {
        cfg.detection_disable_ble_mask = detection_disable_ble_mask;
        let _ = cfg.save(nvs);
    }
    if cfg.remote_id_disable_wifi_mask != remote_id_disable_mask {
        cfg.remote_id_disable_wifi_mask = remote_id_disable_mask;
        let _ = cfg.save(nvs);
    }
    if cfg.remote_id_disable_ble_mask != remote_id_disable_mask {
        cfg.remote_id_disable_ble_mask = remote_id_disable_mask;
        let _ = cfg.save(nvs);
    }
    let suppressed = wardrive_suppressed != 0;
    if cfg.wardrive_suppressed != suppressed {
        info!("sniffer: wardrive_suppressed={}", suppressed as u8);
        cfg.wardrive_suppressed = suppressed;
    }
    let ble_active = ble_active_scan != 0;
    if cfg.ble_active_scan != ble_active {
        info!("sniffer: ble_active_scan={}", ble_active as u8);
        cfg.ble_active_scan = ble_active;
        let _ = cfg.save(nvs);
    }
}

pub(crate) struct SnifferCtx<'a> {
    pub nvs: &'a EspNvsPartition<NvsDefault>,
    pub cfg: &'a mut SnifferConfig,
    pub control_mac: &'a mut Option<[u8; 6]>,
    pub schedule: &'a mut ChannelSchedule,
    pub state: &'a mut SnifferState,
    pub last_cmd_seq: &'a mut u32,
    pub control_established: &'a mut bool,
    pub last_control_rx_ms: &'a mut Option<u64>,
    pub last_control_boot_id: &'a mut Option<u32>,
    pub sniffer_id: u32,
    pub bssid_denylist: &'a mut HashSet<[u8; 6]>,
    pub ssid_denylist: &'a mut HashSet<String>,
    /// Combined SSID watch list (exact + patterns).
    pub ssid_watch: &'a mut SsidWatch,
    pub alerted_probe_srcs: &'a mut HashSet<[u8; 6]>,
    pub last_time_set_seq: &'a mut u32,
    pub pending_time_set: &'a mut Option<proto::TimeSetV1>,
    /// Wall `now_ms` when last unicast RTT `TimeSet` was applied (0 = never).
    pub last_rtt_time_set_ms: &'a mut u64,
    pub pending_ack: &'a mut bool,
    /// Dashboard fleet table slot from [`FrameType::ControlWelcome`] (0 = unknown / DHCP).
    pub fleet_slot: &'a mut u8,
}

impl SnifferCtx<'_> {
    pub fn flush_pending_time_set(&mut self) {
        flush_pending_time_set_standalone(
            self.pending_time_set,
            self.last_time_set_seq,
            *self.last_rtt_time_set_ms,
        );
    }

    fn try_queue_coarse_time_set(&mut self, ts: proto::TimeSetV1) {
        let now_ms = wardriver_stats::now_ms();
        let local_unix = sniffer_unix_now_ms();
        if proto::should_apply_coarse_time_set(
            *self.last_rtt_time_set_ms,
            now_ms,
            local_unix,
            ts.ctrl_unix_ms,
        ) {
            *self.pending_time_set = Some(ts);
        }
    }

    fn reject_foreign_if_bound(&self, src: [u8; 6]) -> bool {
        if let Some(bound) = self.cfg.bound_control_mac {
            if src != bound {
                wardriver_stats::SNIFFER_FOREIGN_FRAME_DROP.fetch_add(1, Ordering::Relaxed);
                return true;
            }
        }
        false
    }

    fn bind_fleet_from_welcome(
        &mut self,
        src: [u8; 6],
        w: &proto::ControlWelcomeV1,
        control_channel: u8,
    ) {
        let first_bind = self.cfg.bound_control_mac.is_none();
        self.cfg
            .set_fleet_binding(src, w.fleet_slot, w.control_boot_id);
        *self.control_mac = Some(src);
        let _ = self.cfg.save(self.nvs);
        let _ = espnow::add_peer(
            src,
            control_channel.max(1),
            esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
        );
        if first_bind {
            info!(
                "sniffer: fleet bound to {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} slot={}",
                src[0], src[1], src[2], src[3], src[4], src[5], w.fleet_slot
            );
        }
    }

    fn handle_release_binding(&mut self) {
        crate::sniffer_link::apply_local_fleet_unbind(
            &mut self.cfg,
            self.nvs,
            self.control_mac,
            self.control_established,
            self.last_control_boot_id,
            self.fleet_slot,
        );
    }

    fn on_control_cmd(&mut self, frame: &espnow::RxFrame, cmd: ControlCmdV1) -> Result<()> {
        let ping = cmd.cmd == ControlCmd::Ping;
        if self.cfg.is_fleet_bound() {
            if *self.control_mac != self.cfg.bound_control_mac {
                *self.control_mac = self.cfg.bound_control_mac;
            }
        } else {
            if ping {
                let _ = espnow::add_peer(
                    frame.src,
                    cmd.control_channel,
                    esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
                );
                if let Some(ts) = proto::time_from_cmd(&cmd) {
                    self.try_queue_coarse_time_set(ts);
                }
            }
            return Ok(());
        }

        let boot_changed = match *self.last_control_boot_id {
            None => false,
            Some(p) => p != cmd.control_boot_id,
        };
        let seq_reboot =
            self.last_control_boot_id.is_some() && *self.last_cmd_seq > 1000 && cmd.cmd_seq < 100;
        if boot_changed || seq_reboot {
            if boot_changed {
                info!(
                    "sniffer: control session changed (boot_id {:?} -> {}); re-adding peer",
                    *self.last_control_boot_id, cmd.control_boot_id
                );
            } else {
                info!("sniffer: control cmd_seq reset; treating as control reboot");
            }
            let _ = espnow::add_peer(
                frame.src,
                cmd.control_channel,
                esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
            );
        }
        *self.last_control_boot_id = Some(cmd.control_boot_id);
        *self.last_control_rx_ms = Some(wardriver_stats::now_ms());

        if !crate::sniffer_link::link_reset_pending() {
            *self.control_established = true;
        }

        *self.last_cmd_seq = cmd.cmd_seq;
        apply_policy_fields(
            self.cfg,
            self.nvs,
            cmd.detection_disable_wifi_mask,
            cmd.detection_disable_ble_mask,
            cmd.remote_id_disable_mask,
            cmd.wardrive_suppressed,
            cmd.ble_active_scan,
        );

        if ping {
            crate::sniffer_dispatch::on_control_ping_hints(
                cmd.peer_slots_free,
                cmd.peer_join_waiters,
            );
            if *self.state != SnifferState::ReconCapturing {
                if cmd.desired_scanning != 0 {
                    if self.cfg.role == SnifferRole::Wifi as u8 && self.schedule.chan_count == 0 {
                        *self.schedule = ChannelSchedule::default();
                    }
                    *self.state = SnifferState::Scanning;
                } else {
                    *self.state = SnifferState::Idle;
                }
            }
        } else {
            match cmd.cmd {
                ControlCmd::StartScanning => {
                    if self.cfg.role == SnifferRole::Wifi as u8 {
                        *self.schedule = ChannelSchedule::from_cmd(cmd);
                    } else {
                        self.schedule.control_channel = cmd.control_channel.max(1);
                        self.schedule.dwell_ms =
                            cmd.dwell_ms.max(50).min(sniffer_sweep::MAX_DWELL_MS);
                        self.schedule.clear_wifi_hop_channels();
                    }
                    *self.state = SnifferState::Scanning;
                }
                ControlCmd::StopScanning => {
                    *self.state = SnifferState::Idle;
                }
                ControlCmd::SetChannelSchedule => {
                    if self.cfg.role == SnifferRole::Wifi as u8 {
                        *self.schedule = ChannelSchedule::from_cmd(cmd);
                    } else if cmd.chan_count == 0 {
                        self.schedule.control_channel = cmd.control_channel.max(1);
                        self.schedule.dwell_ms =
                            cmd.dwell_ms.max(50).min(sniffer_sweep::MAX_DWELL_MS);
                        self.schedule.clear_wifi_hop_channels();
                    }
                }
                ControlCmd::SetRole => {
                    let want = SnifferRole::from_wire(cmd.desired_role);
                    match want {
                        SnifferRole::Wifi => {
                            self.cfg.role = SnifferRole::Wifi as u8;
                            let _ = self.cfg.save(self.nvs);
                            if *self.state == SnifferState::Error {
                                *self.state = SnifferState::Idle;
                            }
                        }
                        SnifferRole::Ble => {
                            #[cfg(feature = "ble")]
                            {
                                self.cfg.role = SnifferRole::Ble as u8;
                                let _ = self.cfg.save(self.nvs);
                                self.schedule.clear_wifi_hop_channels();
                                if *self.state == SnifferState::Error {
                                    *self.state = SnifferState::Idle;
                                }
                                *self.pending_ack = true;
                            }
                            #[cfg(not(feature = "ble"))]
                            {
                                *self.state = SnifferState::Error;
                                warn!(
                                    "sniffer: SetRole(BLE) rejected: firmware built without `ble`"
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        if let Some(ts) = proto::time_from_cmd(&cmd) {
            self.try_queue_coarse_time_set(ts);
        }
        if *self.control_established {
            if let Some(_mac) = *self.control_mac {
                *self.pending_ack = true;
            }
        }
        Ok(())
    }

    fn on_frame(&mut self, frame: espnow::RxFrame) -> Result<()> {
        let data = frame.payload();
        if data.len() < 2 || data[0] != proto::PROTO_VERSION {
            return Ok(());
        }
        if self.reject_foreign_if_bound(frame.src) {
            return Ok(());
        }
        if data[1] == FrameType::ControlCmd as u8 {
            if data.len() >= 15 && data[10] == ControlCmd::ReleaseBinding as u8 {
                if self.cfg.is_fleet_bound() {
                    if let Ok(_epoch) = proto::unpack_release_binding(data) {
                        self.handle_release_binding();
                    }
                }
                return Ok(());
            }
            if data.len() >= 15 && data[10] == ControlCmd::Ping as u8 {
                if let Ok(cmd) = proto::unpack_ping(data) {
                    self.on_control_cmd(&frame, cmd)?;
                }
                return Ok(());
            }
            if let Ok(cmd) = proto::unpack_control_cmd(data) {
                self.on_control_cmd(&frame, cmd)?;
            }
        } else if data[1] == FrameType::ControlWelcome as u8 {
            if let Ok(w) = proto::unpack_control_welcome(data) {
                if self.cfg.bound_control_mac.is_none()
                    || self.cfg.bound_control_mac == Some(frame.src)
                {
                    self.bind_fleet_from_welcome(frame.src, &w, w.control_channel);
                }
                *self.last_control_boot_id = Some(w.control_boot_id);
                *self.control_established = true;
                crate::sniffer_link::clear_link_reset_pending();
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());

                apply_policy_fields(
                    self.cfg,
                    self.nvs,
                    w.detection_disable_wifi_mask,
                    w.detection_disable_ble_mask,
                    w.remote_id_disable_mask,
                    w.wardrive_suppressed,
                    w.ble_active_scan,
                );
                // Apply bootstrap schedule if provided (WiFi role only).
                if self.cfg.role == SnifferRole::Wifi as u8 && w.chan_count > 0 {
                    *self.schedule = ChannelSchedule {
                        control_channel: w.control_channel.max(1),
                        dwell_ms: w.dwell_ms.max(50).min(sniffer_sweep::MAX_DWELL_MS),
                        chan_count: w.chan_count.min(wardriver_proto::CHANNELS_MAX as u8),
                        channels: w.channels,
                    };
                } else {
                    self.schedule.control_channel = w.control_channel.max(1);
                    self.schedule.dwell_ms = w.dwell_ms.max(50).min(sniffer_sweep::MAX_DWELL_MS);
                    self.schedule.clear_wifi_hop_channels();
                }
                *self.state = if w.desired_scanning != 0 {
                    SnifferState::Scanning
                } else {
                    SnifferState::Idle
                };

                if w.fleet_slot != *self.fleet_slot {
                    info!("sniffer: fleet_slot={}", w.fleet_slot);
                }
                *self.fleet_slot = w.fleet_slot;

                *self.pending_ack = true;
            }
        } else if data[1] == FrameType::BssidDenylist as u8 {
            if self.cfg.role == SnifferRole::Ble as u8 {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                return Ok(());
            }
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Ok(macs) = proto::unpack_bssid_denylist(data) {
                *self.bssid_denylist = macs.into_iter().collect();
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                info!(
                    "sniffer: BSSID denylist updated ({} MACs)",
                    self.bssid_denylist.len()
                );
            }
        } else if data[1] == FrameType::SsidDenylist as u8 {
            if self.cfg.role == SnifferRole::Ble as u8 {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                return Ok(());
            }
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Ok(ssids) = proto::unpack_ssid_denylist(data) {
                *self.ssid_denylist = ssids.into_iter().collect();
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                info!(
                    "sniffer: SSID denylist updated ({} SSIDs)",
                    self.ssid_denylist.len()
                );
            }
        } else if data[1] == FrameType::SsidWatchlist as u8 {
            if self.cfg.role == SnifferRole::Ble as u8 {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                return Ok(());
            }
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Ok(ssids) = proto::unpack_ssid_watchlist(data) {
                *self.ssid_watch = SsidWatch::from_strings(ssids);
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                info!(
                    "sniffer: SSID watch updated ({} exact, {} patterns)",
                    self.ssid_watch.exact.len(),
                    self.ssid_watch.patterns.len()
                );
            }
        } else if data[1] == FrameType::TimeSyncPing as u8 {
            if let Some(saved) = *self.control_mac {
                if frame.src != saved {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            if let Ok(ping) = proto::unpack_time_sync_ping(data) {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                let pong = proto::TimeSyncPongV1 {
                    seq: ping.seq,
                    sniff_rx_ms: wardriver_stats::now_ms() as u32,
                    sniffer_id: self.sniffer_id,
                };
                if let Some(mac) = *self.control_mac {
                    let payload = proto::pack_time_sync_pong(pong);
                    let _ = espnow::send(mac, &payload);
                }
            }
        } else if data[1] == FrameType::TimeSet as u8 {
            if let Some(saved) = *self.control_mac {
                if frame.src != saved {
                    return Ok(());
                }
            } else {
                // Don't apply time until control is established/paired.
                return Ok(());
            }
            if let Ok(ts) = proto::unpack_time_set(data) {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                // Unicast RTT-corrected TimeSet: apply immediately (post-dump control window).
                if ts.seq == *self.last_time_set_seq {
                    return Ok(());
                }
                let seq = ts.seq;
                match apply_time_set(ts) {
                    TimeSetApplyOutcome::Applied => {
                        // Only a real settimeofday counts as RTT sync for coarse suppress.
                        *self.last_time_set_seq = seq;
                        *self.last_rtt_time_set_ms = wardriver_stats::now_ms();
                        *self.pending_time_set = None;
                    }
                    TimeSetApplyOutcome::RejectedEpoch => {
                        // Do not bump last_rtt_time_set_ms: a rejected epoch must not block coarse.
                    }
                    TimeSetApplyOutcome::SettimeofdayFailed(rc) => {
                        warn!("sniffer: apply TimeSet failed (non-fatal): settimeofday rc={rc}");
                    }
                }
            }
        } else if data[1] == FrameType::ReconCmd as u8 {
            if let Some(saved) = *self.control_mac {
                if frame.src != saved {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            if let Ok(cmd) = proto::unpack_recon_cmd(data) {
                *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                *self.last_control_boot_id = Some(cmd.control_boot_id);
                match cmd.op {
                    proto::ReconCmdOp::Start => {
                        if cmd.job_kind != proto::ReconJobKind::WifiProbePcap {
                            return Ok(());
                        }
                        if self.cfg.role != SnifferRole::Wifi as u8 {
                            return Ok(());
                        }
                        if cmd.channel_list_revision != proto::RECON_WIFI_CHANNEL_LIST_REVISION {
                            warn!(
                                "sniffer: recon channel list revision {} != {}",
                                cmd.channel_list_revision,
                                proto::RECON_WIFI_CHANNEL_LIST_REVISION
                            );
                            return Ok(());
                        }
                        if let Ok(mut g) = RECON_SESSION.lock() {
                            *g = Some(ReconSessionParams {
                                session_id: cmd.session_id,
                                end_mono_ms: cmd.end_mono_ms,
                                dwell_ms: cmd.dwell_ms.max(40).min(sniffer_sweep::MAX_DWELL_MS),
                                control_ch: cmd.control_channel.max(1),
                            });
                        }
                        *self.state = SnifferState::ReconCapturing;
                    }
                    proto::ReconCmdOp::Abort => {
                        if let Ok(mut g) = RECON_SESSION.lock() {
                            if g.map(|p| p.session_id) == Some(cmd.session_id) {
                                *g = None;
                                *self.state = SnifferState::Idle;
                            }
                        }
                    }
                }
                if *self.control_established {
                    if let Some(_mac) = *self.control_mac {
                        *self.pending_ack = true;
                    }
                }
            }
        } else if data[1] == FrameType::PeerJoinNack as u8 {
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Ok(nack) = proto::unpack_peer_join_nack(data) {
                if nack.sniffer_id == self.sniffer_id {
                    crate::sniffer_dispatch::on_peer_join_nack(
                        self.sniffer_id,
                        nack.retry_after_ms,
                        nack.queue_position,
                        nack.reason,
                    );
                    *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                }
            }
        } else if data[1] == FrameType::PeerJoinAck as u8 {
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Some(bound) = self.cfg.bound_control_mac {
                if frame.src != bound {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            if let Ok(ack) = proto::unpack_peer_join_ack(data) {
                if ack.sniffer_id == self.sniffer_id {
                    crate::sniffer_dispatch::on_peer_join_ack(
                        ack.sniffer_id,
                        frame.src,
                        self.schedule.control_channel,
                    );
                    *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                }
            }
        } else if data[1] == FrameType::PostDumpDone as u8 {
            if !self.cfg.is_fleet_bound() {
                return Ok(());
            }
            if let Some(bound) = self.cfg.bound_control_mac {
                if frame.src != bound {
                    return Ok(());
                }
            } else {
                return Ok(());
            }
            if let Ok(done) = proto::unpack_post_dump_done(data) {
                if done.sniffer_id == self.sniffer_id {
                    crate::sniffer_dispatch::on_post_dump_done(done.session_seq);
                    *self.last_control_rx_ms = Some(wardriver_stats::now_ms());
                }
            }
        }
        Ok(())
    }

    pub fn drain_rx(&mut self, rx: &EspnowRxReceiver<espnow::RxFrame>) -> Result<()> {
        while let Ok(frame) = rx.try_recv() {
            wardriver_stats::queue_inflight_dec(&wardriver_stats::ESPNOW_RX_QUEUE_INFLIGHT);
            self.on_frame(frame)?;
        }
        Ok(())
    }

    /// Pacing sleep after a sweep while still draining RX (fast control recovery traffic).
    pub fn sleep_pacing_with_rx(
        &mut self,
        pacing_ms: u64,
        rx: &EspnowRxReceiver<espnow::RxFrame>,
    ) -> Result<()> {
        // TX power is capped once at boot (see `wifi_tx_power::apply_default_cap_once`).
        const SLICE_MS: u64 = 50;
        let mut rem = pacing_ms;
        while rem > 0 {
            let step = SLICE_MS.min(rem);
            self.drain_rx(rx)?;
            wardriver_shared::task_wdt::reset_if_subscribed();
            std::thread::sleep(Duration::from_millis(step));
            rem -= step;
        }
        Ok(())
    }
}

/// Owned session state for the sniffer main loop. The main loop constructs this once and
/// borrows `&mut` sub-ticks from it via [`SnifferSession::as_ctx`]; fields it touches directly
/// (outside a sub-tick) are read/written as `session.<field>`.
pub(crate) struct SnifferSession<'a> {
    pub nvs: &'a EspNvsPartition<NvsDefault>,
    pub cfg: SnifferConfig,
    pub control_mac: Option<[u8; 6]>,
    pub schedule: ChannelSchedule,
    pub state: SnifferState,
    pub last_cmd_seq: u32,
    pub control_established: bool,
    pub last_control_rx_ms: Option<u64>,
    pub last_control_boot_id: Option<u32>,
    pub sniffer_id: u32,
    pub bssid_denylist: HashSet<[u8; 6]>,
    pub ssid_denylist: HashSet<String>,
    /// Combined SSID watch list (exact + patterns).
    pub ssid_watch: SsidWatch,
    pub alerted_probe_srcs: HashSet<[u8; 6]>,
    pub last_time_set_seq: u32,
    pub pending_time_set: Option<proto::TimeSetV1>,
    /// Wall `now_ms` when last unicast RTT `TimeSet` was applied (0 = never).
    pub last_rtt_time_set_ms: u64,
    pub pending_ack: bool,
    /// Dashboard fleet table slot from [`FrameType::ControlWelcome`] (0 = unknown / DHCP).
    pub fleet_slot: u8,
}

impl SnifferSession<'_> {
    /// Borrow the session as the `&mut`-field [`SnifferCtx`] the sub-ticks consume.
    pub fn as_ctx(&mut self) -> SnifferCtx<'_> {
        SnifferCtx {
            nvs: self.nvs,
            cfg: &mut self.cfg,
            control_mac: &mut self.control_mac,
            schedule: &mut self.schedule,
            state: &mut self.state,
            last_cmd_seq: &mut self.last_cmd_seq,
            control_established: &mut self.control_established,
            last_control_rx_ms: &mut self.last_control_rx_ms,
            last_control_boot_id: &mut self.last_control_boot_id,
            sniffer_id: self.sniffer_id,
            bssid_denylist: &mut self.bssid_denylist,
            ssid_denylist: &mut self.ssid_denylist,
            ssid_watch: &mut self.ssid_watch,
            alerted_probe_srcs: &mut self.alerted_probe_srcs,
            last_time_set_seq: &mut self.last_time_set_seq,
            pending_time_set: &mut self.pending_time_set,
            last_rtt_time_set_ms: &mut self.last_rtt_time_set_ms,
            pending_ack: &mut self.pending_ack,
            fleet_slot: &mut self.fleet_slot,
        }
    }
}

fn should_apply_coarse_time_set(last_rtt_ms: u64, now_ms: u64, offered_unix_ms: u64) -> bool {
    proto::should_apply_coarse_time_set(last_rtt_ms, now_ms, sniffer_unix_now_ms(), offered_unix_ms)
}

pub(crate) fn flush_pending_time_set_standalone(
    pending: &mut Option<proto::TimeSetV1>,
    last_seq: &mut u32,
    last_rtt_time_set_ms: u64,
) {
    let Some(ts) = pending.take() else {
        return;
    };
    let now_ms = wardriver_stats::now_ms();
    if !should_apply_coarse_time_set(last_rtt_time_set_ms, now_ms, ts.ctrl_unix_ms) {
        // Re-queue so a later flush can apply after skew bypass or suppress window expiry.
        *pending = Some(ts);
        return;
    }
    if ts.seq == *last_seq {
        return;
    }
    let seq = ts.seq;
    match apply_time_set(ts) {
        TimeSetApplyOutcome::Applied => {
            *last_seq = seq;
        }
        TimeSetApplyOutcome::RejectedEpoch => {
            // Leave seq unchanged so the next piggyback with a new seq can apply.
        }
        TimeSetApplyOutcome::SettimeofdayFailed(rc) => {
            warn!("sniffer: apply TimeSet failed (non-fatal): settimeofday rc={rc}");
        }
    }
}

/// Result of attempting to apply a control-offered wall clock to the sniffer.
type TimeSetApplyOutcome = wardriver_core::sniffer_time_set::TimeSetApplyOutcome;

fn apply_time_set(ts: proto::TimeSetV1) -> TimeSetApplyOutcome {
    // `ctrl_unix_ms` is already aged on the control CPU to ~send time (PROTO v18+). Do not add
    // sniffer_mono − ctrl_tx_mono_ms32: those counters are different devices' esp_timer bases.
    let unix_ms = ts.ctrl_unix_ms;

    // Basic sanity: ignore obviously invalid epochs (e.g., 1970 after boot).
    if unix_ms < proto::TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS {
        return TimeSetApplyOutcome::RejectedEpoch;
    }

    let tv = sys::timeval {
        tv_sec: (unix_ms / 1000) as sys::time_t,
        tv_usec: ((unix_ms % 1000) * 1000) as sys::suseconds_t,
    };
    let rc = unsafe { sys::settimeofday(&tv, core::ptr::null()) };
    wardriver_core::sniffer_time_set::decide_time_set_apply(unix_ms, rc)
}
