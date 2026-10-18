//! Gateway WiFi probe capture orchestration (MVP foundation).
//!
//! **Control node (full path):** [`wardriver_control::recon`] opens a merged pcapng on SD,
//! reassembles [`wardriver_proto::FrameType::ReconWifiFrag`] into EPB blocks, and exposes
//! download via HTTP.
//!
//! **Gateway MVP stub:** accepts PackLink [`CaptureCmdV1`], fans out wolfPACK
//! [`ReconCmdV1`] to WiFi sniffers, holds fleet scanning, counts frags, and appends
//! reassembled MPDU bytes to a RAM ring buffer. Output is **not** a valid pcapng file yet
//! (no SHB/IDB/EPB merge). Phone receives [`CaptureStatusV1`] progress on `event_notify`;
//! PCAP download over BLE is a follow-up.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use log::{info, warn};
use wardriver_fleet_realtime::gateway_downlink::{
    self, ensure_sniffer_peer_for_unicast, FleetCommandBuilder,
};
use wardriver_fleet_realtime::ESPNOW_PEER_IF;
use wardriver_packlink::{
    capture_error, capture_job_id, pack_capture_status, unpack_capture_cmd, CaptureCmdV1,
    CaptureStatusV1,
};
use wardriver_proto::ControlCmd;
use wardriver_proto::{
    pack_recon_cmd, unpack_recon_wifi_frag, ReconCmdOp, ReconCmdV1, ReconJobKind, SnifferRole,
    RECON_WIFI_CHANNEL_LIST_REVISION, RECON_WIFI_FRAG_MAX_PAYLOAD,
};
use wardriver_shared::espnow;
use wardriver_stats as stats;

use crate::policy_downlink::GatewayFleetState;
use crate::softap_transport;

/// Default capture window when phone sends `duration_sec = 0`.
const DEFAULT_DURATION_SEC: u16 = 60;
/// Max capture window accepted from phone.
const MAX_DURATION_SEC: u16 = 120;
/// RAM buffer cap for stub capture payload.
const CAPTURE_RAM_CAP: usize = 200 * 1024;
const DWELL_MS: u16 = 120;
/// Minimum interval between `CaptureStatus` BLE notifies.
const STATUS_NOTIFY_INTERVAL_MS: u64 = 500;

struct ReconPartial {
    frag_total: u8,
    mpdu_len: u16,
    buf: Vec<u8>,
    got: u32,
}

struct ActiveCapture {
    session_id: u32,
    end_mono_ms: u64,
    duration_ms: u32,
    control_boot_id: u32,
    control_ch: u8,
    /// Raw reassembled MPDU bytes (MVP stub — not pcapng EPB records).
    ram_buf: Vec<u8>,
    partial: HashMap<(u32, u32), ReconPartial>,
    if_by_sniffer: HashMap<u32, u32>,
    last_error: u8,
}

static ACTIVE: OnceLock<Mutex<Option<ActiveCapture>>> = OnceLock::new();
static STATUS_SEQ: AtomicU32 = AtomicU32::new(1);
static LAST_STATUS_NOTIFY_MS: AtomicU32 = AtomicU32::new(0);

fn active_slot() -> &'static Mutex<Option<ActiveCapture>> {
    ACTIVE.get_or_init(|| Mutex::new(None))
}

/// Handle PackLink `CaptureCmd` write on `control_cmd` characteristic.
pub fn on_capture_cmd(cmd: &CaptureCmdV1, fleet: &GatewayFleetState) -> Result<()> {
    if cmd.job_id != capture_job_id::WIFI_PROBE_PCAP {
        emit_status(idle_status(capture_error::UNSUPPORTED_JOB));
        return Ok(());
    }
    if active_slot().lock().map(|g| g.is_some()).unwrap_or(false) {
        emit_status(idle_status(capture_error::BUSY));
        return Ok(());
    }

    let duration_sec = if cmd.duration_sec == 0 {
        DEFAULT_DURATION_SEC
    } else {
        cmd.duration_sec.min(MAX_DURATION_SEC)
    };
    let duration_ms = u32::from(duration_sec) * 1000;
    let now_ms = stats::now_ms();
    let end_mono_ms = now_ms.saturating_add(u64::from(duration_ms));

    let snap = stats::snapshot_sniffers();
    let mut wifi: Vec<(u32, [u8; 6])> = Vec::new();
    for s in snap.iter() {
        if s.sniffer_id == 0 || s.role != SnifferRole::Wifi as u8 {
            continue;
        }
        if now_ms.saturating_sub(s.last_seen_ms) > 120_000 {
            continue;
        }
        if s.mac == [0u8; 6] {
            continue;
        }
        wifi.push((s.sniffer_id, s.mac));
    }
    wifi.sort_by_key(|w| w.0);
    if wifi.is_empty() {
        emit_status(idle_status(capture_error::NO_SNIFFERS));
        return Ok(());
    }

    // Hold fleet scanning while capture runs (mirrors control `recon_hold`).
    stats::set_recon_hold(true);
    stats::RECON_ACTIVE.store(1, Ordering::Release);
    stats::set_recon_timing_ms(end_mono_ms, duration_ms);

    let builder = fleet.cmd_builder();
    if let Err(e) = gateway_downlink::broadcast_cmd(
        &fleet.sniffers,
        &fleet.cmd_seq,
        ControlCmd::StopScanning,
        None,
        ESPNOW_PEER_IF,
        fleet.control_boot_id.load(Ordering::Relaxed),
        true,
        &builder,
        Some(stats::time_set_seq_for_wire()),
        0,
    ) {
        warn!("gateway capture: hold broadcast StopScanning: {e:#}");
    }

    let session_id = (unsafe { esp_idf_sys::esp_random() } & 0x7fff_ffff).max(1);
    stats::RECON_SESSION_ID.store(session_id, Ordering::Release);
    stats::RECON_FRAGS_RX.store(0, Ordering::Release);
    stats::RECON_PKTS_WRITTEN.store(0, Ordering::Release);
    stats::RECON_REASM_FAIL.store(0, Ordering::Release);

    let control_ch = wardriver_fleet_realtime::CONTROL_AP_CHANNEL.max(1);
    let recon_cmd = ReconCmdV1 {
        op: ReconCmdOp::Start,
        job_kind: ReconJobKind::WifiProbePcap,
        session_id,
        duration_ms,
        end_mono_ms,
        dwell_ms: DWELL_MS,
        channel_list_revision: RECON_WIFI_CHANNEL_LIST_REVISION,
        phase_offset: 0,
        control_channel: control_ch,
        control_boot_id: fleet.control_boot_id.load(Ordering::Relaxed),
    };
    let payload = pack_recon_cmd(recon_cmd);
    for (_, mac) in &wifi {
        if let Err(e) = gateway_downlink::ensure_sniffer_peer_for_unicast(
            *mac,
            ESPNOW_PEER_IF,
            "gateway capture recon start",
        ) {
            warn!("gateway capture: {e:#}");
            continue;
        }
        if let Err(e) = espnow::send(*mac, &payload) {
            warn!("gateway capture: recon start to {:02x?}: {e:#}", mac);
        }
        if let Ok(mut g) = fleet.sniffers.lock() {
            g.insert(*mac);
        }
    }

    let mut if_by = HashMap::new();
    for (i, (sid, _)) in wifi.iter().enumerate() {
        if_by.insert(*sid, i as u32);
    }

    if let Ok(mut g) = active_slot().lock() {
        *g = Some(ActiveCapture {
            session_id,
            end_mono_ms,
            duration_ms,
            control_boot_id: fleet.control_boot_id.load(Ordering::Relaxed),
            control_ch,
            ram_buf: Vec::new(),
            partial: HashMap::new(),
            if_by_sniffer: if_by,
            last_error: capture_error::NONE,
        });
    }

    info!(
        "gateway capture: started session_id={} sniffers={} duration_ms={}",
        session_id,
        wifi.len(),
        duration_ms
    );
    emit_status(current_status());
    Ok(())
}

/// Decode PackLink frame bytes from `control_cmd` write; returns true when handled.
pub fn try_handle_control_cmd_write(data: &[u8], fleet: &GatewayFleetState) -> Result<bool> {
    let frame = wardriver_packlink::unpack_frame(data)?;
    if frame.msg_type != wardriver_packlink::MsgType::CaptureCmd {
        return Ok(false);
    }
    let cmd = unpack_capture_cmd(frame.payload)?;
    on_capture_cmd(&cmd, fleet)?;
    Ok(true)
}

/// Ingest one wolfPACK `ReconWifiFrag` (MVP: count + append raw MPDU to RAM stub).
pub fn ingest_wifi_frag(data: &[u8], last_sniffer_boot_id: &HashMap<u32, u32>) -> Result<()> {
    let (hdr, chunk) = unpack_recon_wifi_frag(data)?;
    if hdr.frag_total == 0 || hdr.frag_total > 31 {
        stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    stats::RECON_FRAGS_RX.fetch_add(1, Ordering::Relaxed);

    let mut guard = active_slot()
        .lock()
        .map_err(|_| anyhow!("capture active mutex poison"))?;
    let Some(act) = guard.as_mut() else {
        return Ok(());
    };
    if hdr.session_id != act.session_id {
        return Ok(());
    }
    if let Some(&boot) = last_sniffer_boot_id.get(&hdr.sniffer_id) {
        if boot != hdr.sniffer_boot_id {
            return Ok(());
        }
    }
    if !act.if_by_sniffer.contains_key(&hdr.sniffer_id) {
        stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    let key = (hdr.sniffer_id, hdr.capture_seq);
    let maxc = RECON_WIFI_FRAG_MAX_PAYLOAD;
    let off = (hdr.frag_idx as usize).saturating_mul(maxc);
    let complete_mpdu = match act.partial.entry(key) {
        std::collections::hash_map::Entry::Vacant(v) => {
            if hdr.frag_idx != 0 {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            let mut buf = vec![0u8; hdr.mpdu_total_len as usize];
            if off + chunk.len() > buf.len() {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            buf[off..off + chunk.len()].copy_from_slice(chunk);
            let mask = 1u32 << hdr.frag_idx;
            let done = hdr.frag_total == 1;
            if done {
                Some(buf)
            } else {
                v.insert(ReconPartial {
                    frag_total: hdr.frag_total,
                    mpdu_len: hdr.mpdu_total_len,
                    buf,
                    got: mask,
                });
                None
            }
        }
        std::collections::hash_map::Entry::Occupied(mut o) => {
            let p = o.get_mut();
            if p.frag_total != hdr.frag_total || p.mpdu_len != hdr.mpdu_total_len {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                o.remove();
                return Ok(());
            }
            if off + chunk.len() > p.buf.len() {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                o.remove();
                return Ok(());
            }
            p.buf[off..off + chunk.len()].copy_from_slice(chunk);
            p.got |= 1u32 << hdr.frag_idx;
            let need = (1u32 << p.frag_total) - 1;
            if p.got == need {
                let buf = o.remove().buf;
                Some(buf)
            } else {
                None
            }
        }
    };

    if let Some(mpdu) = complete_mpdu {
        // MVP stub: append raw 802.11 MPDU bytes (full pcapng needs EPB headers per interface).
        if act.ram_buf.len().saturating_add(mpdu.len()) > CAPTURE_RAM_CAP {
            act.last_error = capture_error::BUFFER_FULL;
            warn!("gateway capture: RAM buffer full; dropping MPDU");
        } else {
            act.ram_buf.extend_from_slice(&mpdu);
            stats::RECON_PKTS_WRITTEN.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Main-loop tick: end expired sessions, emit periodic `CaptureStatus`.
pub fn poll(fleet: &GatewayFleetState) {
    let now_ms = stats::now_ms();
    let mut ended = false;
    if let Ok(mut g) = active_slot().lock() {
        if let Some(act) = g.as_ref() {
            if now_ms >= act.end_mono_ms.saturating_add(2_000) {
                ended = true;
            }
        }
        if ended {
            if let Some(act) = g.take() {
                finish_session(fleet, &act);
            }
        }
    }

    let active = active_slot().lock().map(|g| g.is_some()).unwrap_or(false);
    if active || ended {
        let last = u64::from(LAST_STATUS_NOTIFY_MS.load(Ordering::Relaxed));
        if now_ms.saturating_sub(last) >= STATUS_NOTIFY_INTERVAL_MS || ended {
            LAST_STATUS_NOTIFY_MS.store(now_ms.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
            emit_status(current_status());
        }
    }
}

fn finish_session(fleet: &GatewayFleetState, act: &ActiveCapture) {
    let abort = pack_recon_cmd(ReconCmdV1 {
        op: ReconCmdOp::Abort,
        job_kind: ReconJobKind::WifiProbePcap,
        session_id: act.session_id,
        duration_ms: act.duration_ms,
        end_mono_ms: act.end_mono_ms,
        dwell_ms: DWELL_MS,
        channel_list_revision: RECON_WIFI_CHANNEL_LIST_REVISION,
        phase_offset: 0,
        control_channel: act.control_ch,
        control_boot_id: act.control_boot_id,
    });
    if let Ok(peers) = fleet.sniffers.lock() {
        for mac in peers.iter() {
            if let Err(e) =
                ensure_sniffer_peer_for_unicast(*mac, ESPNOW_PEER_IF, "gateway capture recon abort")
            {
                warn!("gateway capture: {e:#}");
                continue;
            }
            let _ = espnow::send(*mac, &abort);
        }
    }

    stats::set_recon_hold(false);
    stats::RECON_ACTIVE.store(0, Ordering::Release);
    stats::clear_recon_timing_ms();

    // Resume scanning when phone policy allows.
    let policy = crate::policy_downlink::current_policy(fleet);
    if policy.scan_allowed {
        let schedule = if policy.chan_count > 0 {
            Some((policy.channels, policy.chan_count))
        } else {
            None
        };
        let builder = FleetCommandBuilder::new(policy.dwell_ms);
        if let Err(e) = gateway_downlink::broadcast_cmd(
            &fleet.sniffers,
            &fleet.cmd_seq,
            ControlCmd::StartScanning,
            schedule,
            ESPNOW_PEER_IF,
            fleet.control_boot_id.load(Ordering::Relaxed),
            true,
            &builder,
            Some(stats::time_set_seq_for_wire()),
            1,
        ) {
            warn!("gateway capture: resume StartScanning: {e:#}");
        }
    }

    info!(
        "gateway capture: session {} closed bytes={} frags={}",
        act.session_id,
        act.ram_buf.len(),
        stats::RECON_FRAGS_RX.load(Ordering::Relaxed)
    );
}

#[must_use]
pub fn current_status() -> CaptureStatusV1 {
    let now_ms = stats::now_ms();
    let (end_ms, _duration_ms) = stats::recon_timing_ms();
    let active = active_slot().lock().map(|g| g.is_some()).unwrap_or(false);
    let remaining_ms = if active && end_ms > now_ms {
        (end_ms - now_ms).min(u64::from(u32::MAX)) as u32
    } else {
        0
    };
    let last_error = active_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|a| a.last_error))
        .unwrap_or(capture_error::NONE);
    let bytes_written = active_slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|a| a.ram_buf.len() as u32))
        .unwrap_or(0);
    CaptureStatusV1 {
        active: u8::from(active),
        hold: u8::from(stats::recon_hold()),
        remaining_ms: if active { remaining_ms } else { 0 },
        frags_rx: stats::RECON_FRAGS_RX.load(Ordering::Relaxed),
        bytes_written,
        error_code: if active {
            last_error
        } else {
            capture_error::NONE
        },
    }
}

fn idle_status(error_code: u8) -> CaptureStatusV1 {
    CaptureStatusV1 {
        active: 0,
        hold: u8::from(stats::recon_hold()),
        remaining_ms: 0,
        frags_rx: stats::RECON_FRAGS_RX.load(Ordering::Relaxed),
        bytes_written: stats::RECON_PKTS_WRITTEN.load(Ordering::Relaxed),
        error_code,
    }
}

fn emit_status(status: CaptureStatusV1) {
    let seq = STATUS_SEQ.fetch_add(1, Ordering::Relaxed);
    let frame = pack_capture_status(&status, seq);
    let _ = softap_transport::send_frame(frame);
}
