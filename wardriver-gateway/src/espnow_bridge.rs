//! ESP-NOW RX bridge: decrypt, reassemble, ingress, export to PackLink TCP or PSRAM spool.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver as MpscReceiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::gateway_ingress::{self, GatewayIngressCtx};
use anyhow::Result;
use log::{info, warn};
use wardriver_core::scan_reasm::{
    ingest_ble_scan_frag, ingest_probe_observation_frag, ingest_wifi_scan_frag,
};
use wardriver_fleet_realtime::frame_router::peek_ingress_sniffer_id;
use wardriver_fleet_realtime::ingress_roster_should_drop;
use wardriver_fleet_realtime::peer_pool::{self, IngestPostDumpDeps};
use wardriver_fleet_realtime::roster_gate::roster_allows;
use wardriver_fleet_realtime::time_sync_rtt;
use wardriver_fleet_realtime::ESPNOW_PEER_IF;
use wardriver_packlink::{
    event_kind, flags, pack_event_notify, pack_observation_bulk, pack_spool_replay, EventNotifyV1,
    ObservationBulkV1, SpoolReplayV1,
};
use wardriver_proto::{
    unpack_ble_scan_frag, unpack_detection_alert, unpack_detection_ble_alert,
    unpack_probe_observation_frag, unpack_remote_id_alert, unpack_scan_frag,
    unpack_ssid_watch_alert, unpack_time_sync_pong, FrameType, FRAG_FLAG_TRANSPORT_END, MAGIC,
    PROTO_VERSION,
};
use wardriver_shared::espnow::RxFrame;
use wardriver_shared::spiram_queue::{Receiver, TryRecvError};
use wardriver_shared::spiram_thread;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::fleet_loop::{self, GatewayFleetLoopState};
use crate::ingest_state::GatewayIngestState;
use crate::policy_downlink::{self, GatewayFleetState};
use crate::psram_spool;
use crate::softap_transport;

const SPOOL_CHUNK: usize = 400;
const NOTIFY_FLUSH_MAX_PER_TICK: usize = 16;
const SPOOL_REPLAY_MAX_PER_TICK: usize = 4;
const FLEET_LOOP_PERIOD_MS: u64 = 50;
const RX_BATCH_MAX: usize = 64;
const NOTIFY_MAX_FRAMES: usize = 128;
const NOTIFY_MAX_BYTES: usize = 192 * 1024;
const STATUS_DROP_LOG_PERIOD_MS: u64 = 5_000;

struct SpoolReplayCursor {
    total_bytes: u32,
    tail_epoch: u32,
    snapshot_nonce: u32,
    offset: u32,
    chunk_index: u32,
    chunk_total: u32,
    session_generation: u32,
}

/// Outcome of a single uplink-frag reassembly pass.
enum UplinkFragResult {
    /// Batch incomplete; liveness was noted but nothing to forward.
    Incomplete,
    /// Batch complete; forward `frame` carrying the fragment's transport flags.
    Forward { transport_flags: u8, frame: Vec<u8> },
}

pub struct EspnowBridge {
    rx: Receiver<RxFrame>,
    fleet: Arc<GatewayFleetState>,
    notify_q: VecDeque<Vec<u8>>,
    notify_queued_bytes: usize,
    last_status_drop_log_ms: u64,
    ingest: Arc<Mutex<GatewayIngestState>>,
    last_sniffer_boot_id: HashMap<u32, u32>,
    bulk_seq: u32,
    replay_nonce: u32,
    spool_replay: Option<SpoolReplayCursor>,
    fleet_loop_state: GatewayFleetLoopState,
    last_fleet_loop_ms: u64,
}

impl EspnowBridge {
    pub fn spawn(
        rx: Receiver<RxFrame>,
        fleet: Arc<GatewayFleetState>,
        ingest: Arc<Mutex<GatewayIngestState>>,
    ) -> Result<SyncSender<()>> {
        let (tick_tx, tick_rx) = mpsc::sync_channel::<()>(4);
        let bridge = Self {
            rx,
            fleet,
            notify_q: VecDeque::new(),
            notify_queued_bytes: 0,
            last_status_drop_log_ms: 0,
            ingest,
            last_sniffer_boot_id: HashMap::new(),
            bulk_seq: 0,
            replay_nonce: 0,
            spool_replay: None,
            fleet_loop_state: GatewayFleetLoopState::default(),
            last_fleet_loop_ms: 0,
        };
        spiram_thread::spawn_named(
            "gw-espnow",
            spiram_thread::GATEWAY_ESPNOW_STACK_BYTES,
            move || bridge.run(tick_rx),
        )?;
        Ok(tick_tx)
    }

    fn run(mut self, tick_rx: MpscReceiver<()>) {
        wardriver_shared::task_wdt::register_current_task_for_watchdog();
        info!("gateway espnow bridge started");
        loop {
            let mut batch = Vec::new();
            while batch.len() < RX_BATCH_MAX {
                match self.rx.try_recv() {
                    Ok(frame) => batch.push(frame),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if batch.is_empty() {
                            return;
                        }
                        break;
                    }
                }
            }
            self.process_batch(&batch);
            task_wdt::reset_if_subscribed();
            while tick_rx.try_recv().is_ok() {
                self.flush_notify_queue();
            }
            self.tick_fleet_and_peers();
            task_wdt::sleep_feeding_wdt(Duration::from_millis(5));
        }
    }

    fn process_batch(&mut self, frames: &[RxFrame]) {
        for frame in frames {
            if is_detection_alert_frame(frame) {
                if let Err(e) = self.handle_frame(frame) {
                    warn!("gateway frame: {e:#}");
                }
            }
        }
        for frame in frames {
            if !is_detection_alert_frame(frame) {
                if let Err(e) = self.handle_frame(frame) {
                    warn!("gateway frame: {e:#}");
                }
            }
        }
    }

    fn tick_fleet_and_peers(&mut self) {
        let now_ms = stats::now_ms();
        let _ = peer_pool::tick_visit_arm_expiry(now_ms);
        let _ = peer_pool::tick_join_queue_prune(now_ms);
        let deps = IngestPostDumpDeps {
            cmd_seq: &self.fleet.cmd_seq,
            control_boot_id: self.fleet.control_boot_id.load(Ordering::Relaxed),
            peer_if: ESPNOW_PEER_IF,
            sniffers: &self.fleet.sniffers,
            scanning_allowed: &self.fleet.scanning_allowed,
        };
        peer_pool::tick_ingest_post_dump_phase(&deps);
        if now_ms.saturating_sub(self.last_fleet_loop_ms) >= FLEET_LOOP_PERIOD_MS {
            self.last_fleet_loop_ms = now_ms;
            fleet_loop::tick(&self.fleet, &mut self.fleet_loop_state);
        }
    }

    fn phone_connected(&self) -> bool {
        softap_transport::phone_connected()
    }

    fn track_sniffer_mac(&self, mac: [u8; 6]) {
        if let Ok(mut s) = self.fleet.sniffers.lock() {
            s.insert(mac);
        }
    }

    fn handle_frame(&mut self, frame: &RxFrame) -> Result<()> {
        self.track_sniffer_mac(frame.src);
        let payload = frame.payload();
        if payload.len() < 6 || &payload[2..6] != MAGIC {
            return Ok(());
        }
        if payload[0] != PROTO_VERSION {
            return Ok(());
        }

        let policy = policy_downlink::current_policy(&self.fleet);
        if let Some(sid) = peek_ingress_sniffer_id(payload) {
            let allows = roster_allows(&policy, sid, &frame.src);
            if ingress_roster_should_drop(payload[1], allows) {
                stats::INGRESS_ROSTER_DROP.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }

        let ft = payload[1];
        match ft {
            x if x == FrameType::SnifferHello as u8 => {
                stats::INGRESS_HELLO_RX.fetch_add(1, Ordering::Relaxed);
                if let Some(ensure_peer) = self
                    .with_ingress_mut(|ctx| {
                        gateway_ingress::handle_sniffer_hello(ctx, frame.src, payload)
                    })
                    .flatten()
                {
                    if let Err(e) =
                        wardriver_fleet_realtime::gateway_downlink::ensure_sniffer_peer_for_unicast(
                            ensure_peer,
                            ESPNOW_PEER_IF,
                            "hello path espnow peer sniffer",
                        )
                    {
                        warn!("gateway: {e:#}");
                    }
                }
            }
            x if x == FrameType::PeerJoin as u8 => {
                self.with_ingress(|ctx| {
                    gateway_ingress::handle_peer_join(ctx, frame.src, payload);
                });
            }
            x if x == FrameType::SnifferAck as u8 => {
                self.with_ingress(|ctx| {
                    gateway_ingress::handle_sniffer_ack(ctx, frame.src, payload);
                });
            }
            x if x == FrameType::TimeSyncPong as u8 => {
                if let Ok(pong) = unpack_time_sync_pong(payload) {
                    stats::sniffer_seen_liveness(pong.sniffer_id, &frame.src);
                    time_sync_rtt::handle_time_sync_pong(
                        frame.src,
                        pong,
                        stats::now_ms(),
                        ESPNOW_PEER_IF,
                    );
                }
            }
            x if x == FrameType::ScanFrag as u8 => self.handle_scan_frag(frame.src, payload)?,
            x if x == FrameType::BleScanFrag as u8 => {
                self.handle_ble_scan_frag(frame.src, payload)?
            }
            x if x == FrameType::ProbeReqFrag as u8 => {
                self.handle_probe_req_frag(frame.src, payload)?
            }
            x if x == FrameType::ProbeObservationFrag as u8 => {
                self.handle_probe_observation_frag(frame.src, payload)?
            }
            x if x == FrameType::DetectionAlert as u8 => {
                self.handle_priority_event(event_kind::DETECTION_WIFI, payload, frame.src)?
            }
            x if x == FrameType::DetectionBleAlert as u8 => {
                self.handle_priority_event(event_kind::DETECTION_BLE, payload, frame.src)?
            }
            x if x == FrameType::SsidWatchAlert as u8 => {
                self.handle_priority_event(event_kind::SSID_WATCH, payload, frame.src)?
            }
            x if x == FrameType::RemoteIdAlert as u8 => {
                self.handle_priority_event(event_kind::REMOTE_ID, payload, frame.src)?
            }
            x if x == FrameType::ReconWifiFrag as u8 => {
                if let Err(e) =
                    crate::capture_job::ingest_wifi_frag(payload, &self.last_sniffer_boot_id)
                {
                    warn!("gateway capture frag: {e:#}");
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn with_ingress<F>(&mut self, f: F)
    where
        F: FnOnce(&mut GatewayIngressCtx<'_, GatewayIngestState>),
    {
        let _ = self.with_ingress_mut(|ctx| {
            f(ctx);
        });
    }

    fn with_ingress_mut<F, R>(&mut self, f: F) -> Option<R>
    where
        F: FnOnce(&mut GatewayIngressCtx<'_, GatewayIngestState>) -> R,
    {
        let policy = policy_downlink::current_policy(&self.fleet);
        let Ok(mut ingest_guard) = self.ingest.lock() else {
            return None;
        };
        let mut ctx = GatewayIngressCtx {
            policy: &policy,
            control_boot_id: self.fleet.control_boot_id.load(Ordering::Relaxed),
            cmd_seq: &self.fleet.cmd_seq,
            last_sniffer_boot_id: &mut self.last_sniffer_boot_id,
            ingest: &mut *ingest_guard,
        };
        Some(f(&mut ctx))
    }

    /// Shared uplink-frag shape: lock ingest, build ctx, liveness/boot check, then let
    /// `on_reasm` do the per-kind reassembly + pack (returning [UplinkFragResult]).
    /// The helper owns the common trailing: `note_bulk_frag`, the transport-end bookkeeping,
    /// and `export_frame`. `seq` is the live `bulk_seq` counter, incremented on completion.
    fn ingest_uplink_frag(
        &mut self,
        peer: [u8; 6],
        sniffer_id: u32,
        sniffer_boot_id: u32,
        on_reasm: impl FnOnce(&mut GatewayIngestState, u64, &mut u32) -> UplinkFragResult,
    ) -> Result<()> {
        let (now, result) = {
            let policy = policy_downlink::current_policy(&self.fleet);
            let Ok(mut ingest_guard) = self.ingest.lock() else {
                return Ok(());
            };
            let mut ctx = GatewayIngressCtx {
                policy: &policy,
                control_boot_id: self.fleet.control_boot_id.load(Ordering::Relaxed),
                cmd_seq: &self.fleet.cmd_seq,
                last_sniffer_boot_id: &mut self.last_sniffer_boot_id,
                ingest: &mut *ingest_guard,
            };
            if !gateway_ingress::uplink_liveness_and_boot_ok(
                &mut ctx,
                sniffer_id,
                &peer,
                sniffer_boot_id,
            ) {
                return Ok(());
            }
            let now = stats::now_ms();
            let result = on_reasm(&mut *ingest_guard, now, &mut self.bulk_seq);
            (now, result)
        };
        match result {
            UplinkFragResult::Incomplete => {
                peer_pool::note_bulk_frag(peer, now);
                Ok(())
            }
            UplinkFragResult::Forward {
                transport_flags,
                frame,
            } => {
                peer_pool::note_bulk_frag(peer, now);
                if transport_flags & FRAG_FLAG_TRANSPORT_END != 0 {
                    peer_pool::mark_transport_end(peer, now);
                    peer_pool::enqueue_post_dump_flush(peer);
                }
                self.export_frame(frame, false)
            }
        }
    }

    fn handle_scan_frag(&mut self, peer: [u8; 6], wire: &[u8]) -> Result<()> {
        let Ok((hdr, aps)) = unpack_scan_frag(wire) else {
            stats::INGRESS_UNPACK_FAIL_WIFI.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        self.ingest_uplink_frag(
            peer,
            hdr.sniffer_id,
            hdr.sniffer_boot_id,
            |ingest, now, seq| {
                stats::sniffer_frame(hdr.sniffer_id, aps.len());
                let GatewayIngestState {
                    ref mut reasm_dedup,
                    ref mut reasm_state,
                    ..
                } = ingest;
                let Some((hdr, complete_aps)) =
                    ingest_wifi_scan_frag(hdr, aps, reasm_dedup, reasm_state, now)
                else {
                    return UplinkFragResult::Incomplete;
                };
                *seq = seq.wrapping_add(1);
                let bulk = ObservationBulkV1 {
                    batch_id: hdr.scan_batch_id,
                    sniffer_id: hdr.sniffer_id,
                    scan_end_unix_ms: hdr.sniffer_scan_end_unix_ms,
                    wifi_ap_count: complete_aps.len() as u16,
                    ble_count: 0,
                    payload: wire.to_vec(),
                };
                let frame = pack_observation_bulk(&bulk, *seq, flags::FRAGMENT_END);
                UplinkFragResult::Forward {
                    transport_flags: hdr.flags,
                    frame,
                }
            },
        )
    }

    fn handle_ble_scan_frag(&mut self, peer: [u8; 6], wire: &[u8]) -> Result<()> {
        let Ok((hdr, entries)) = unpack_ble_scan_frag(wire) else {
            stats::INGRESS_UNPACK_FAIL_BLE.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        self.ingest_uplink_frag(
            peer,
            hdr.sniffer_id,
            hdr.sniffer_boot_id,
            |ingest, now, seq| {
                stats::sniffer_ble_frame(hdr.sniffer_id, entries.len());
                let GatewayIngestState {
                    ref mut reasm_ble_dedup,
                    ref mut reasm_ble_state,
                    ..
                } = ingest;
                let Some((hdr, entry)) =
                    ingest_ble_scan_frag(hdr, entries, reasm_ble_dedup, reasm_ble_state, now)
                else {
                    return UplinkFragResult::Incomplete;
                };
                *seq = seq.wrapping_add(1);
                let bulk = ObservationBulkV1 {
                    batch_id: hdr.scan_batch_id,
                    sniffer_id: hdr.sniffer_id,
                    scan_end_unix_ms: hdr.sniffer_scan_end_unix_ms,
                    wifi_ap_count: 0,
                    ble_count: entry.entries.len() as u16,
                    payload: wire.to_vec(),
                };
                let frame = pack_observation_bulk(&bulk, *seq, flags::FRAGMENT_END);
                UplinkFragResult::Forward {
                    transport_flags: hdr.flags,
                    frame,
                }
            },
        )
    }

    fn handle_probe_req_frag(&mut self, peer: [u8; 6], payload: &[u8]) -> Result<()> {
        let Ok((hdr, entries)) = wardriver_proto::unpack_probe_req_frag(payload) else {
            return Ok(());
        };
        stats::sniffer_seen_liveness(hdr.sniffer_id, &peer);
        peer_pool::note_bulk_frag(peer, stats::now_ms());
        stats::DETECTION_PROBE_FRAGS_RX.fetch_add(1, Ordering::Relaxed);
        stats::DETECTION_BURSTS_RX.fetch_add(entries.len() as u32, Ordering::Relaxed);
        Ok(())
    }

    fn handle_probe_observation_frag(&mut self, peer: [u8; 6], wire: &[u8]) -> Result<()> {
        let Ok((hdr, entries)) = unpack_probe_observation_frag(wire) else {
            stats::INGRESS_UNPACK_FAIL_PROBE.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        self.ingest_uplink_frag(
            peer,
            hdr.sniffer_id,
            hdr.sniffer_boot_id,
            |ingest, now, seq| {
                stats::sniffer_frame(hdr.sniffer_id, entries.len());
                let GatewayIngestState {
                    ref mut reasm_probe_dedup,
                    ref mut reasm_probe_state,
                    ..
                } = ingest;
                let Some((hdr, complete_entries)) = ingest_probe_observation_frag(
                    hdr,
                    entries,
                    reasm_probe_dedup,
                    reasm_probe_state,
                    now,
                ) else {
                    return UplinkFragResult::Incomplete;
                };
                *seq = seq.wrapping_add(1);
                let payload_bytes: Vec<u8> = complete_entries
                    .iter()
                    .flat_map(|e| {
                        let mut buf =
                            Vec::with_capacity(wardriver_proto::PROBE_OBSERVATION_WIRE_SIZE);
                        buf.extend_from_slice(&e.src_mac);
                        buf.push(e.ssid_len);
                        buf.extend_from_slice(&e.ssid);
                        buf.push(e.channel);
                        buf.push(e.rssi as u8);
                        buf
                    })
                    .collect();
                let bulk = wardriver_packlink::ProbeObservationBulkV1 {
                    batch_id: hdr.scan_batch_id,
                    sniffer_id: hdr.sniffer_id,
                    scan_end_unix_ms: hdr.sniffer_scan_end_unix_ms as u64,
                    scan_end_ms: hdr.sniffer_scan_end_ms,
                    count: complete_entries.len() as u16,
                    payload: payload_bytes,
                };
                let frame = wardriver_packlink::pack_probe_observation_bulk(
                    &bulk,
                    *seq,
                    flags::FRAGMENT_END,
                );
                UplinkFragResult::Forward {
                    transport_flags: hdr.flags,
                    frame,
                }
            },
        )
    }

    fn handle_priority_event(&mut self, kind: u8, payload: &[u8], peer: [u8; 6]) -> Result<()> {
        let sniffer_id = sniffer_id_from_event_payload(kind, payload);
        if sniffer_id != 0 {
            stats::sniffer_seen_liveness(sniffer_id, &peer);
        }
        let ev = EventNotifyV1 {
            event_kind: kind,
            sniffer_id,
            unix_ms: policy_downlink::gateway_phone_unix_ms_now().unwrap_or(0),
            payload_len: payload.len().min(u16::MAX as usize) as u16,
            payload: payload.to_vec(),
        };
        let frame = pack_event_notify(&ev, self.bulk_seq);
        self.bulk_seq = self.bulk_seq.wrapping_add(1);
        self.export_frame(frame, true)
    }

    fn export_frame(&mut self, frame: Vec<u8>, priority: bool) -> Result<()> {
        if self.phone_connected() {
            if priority {
                if let Err(frame) = softap_transport::try_send_frame(frame) {
                    self.enqueue_notify(frame, true)?;
                }
            } else {
                self.enqueue_notify(frame, false)?;
            }
        } else {
            psram_spool::append(&frame)?;
        }
        Ok(())
    }

    fn replace_notify_sniffer_status(&mut self) {
        if let Some(idx) = self
            .notify_q
            .iter()
            .rposition(|f| softap_frame_type(f) == Some(wardriver_packlink::MsgType::SnifferStatus))
        {
            if let Some(old) = self.notify_q.remove(idx) {
                self.notify_queued_bytes = self.notify_queued_bytes.saturating_sub(old.len());
            }
        }
    }

    fn enqueue_notify(&mut self, frame: Vec<u8>, front: bool) -> Result<()> {
        let fits = self.notify_q.len() < NOTIFY_MAX_FRAMES
            && self.notify_queued_bytes.saturating_add(frame.len()) <= NOTIFY_MAX_BYTES;
        if fits {
            self.notify_queued_bytes = self.notify_queued_bytes.saturating_add(frame.len());
            if front {
                self.notify_q.push_front(frame);
            } else {
                self.notify_q.push_back(frame);
            }
            return Ok(());
        }

        match softap_frame_type(&frame) {
            Some(wardriver_packlink::MsgType::SnifferStatus) => {
                self.replace_notify_sniffer_status();
                let fits_after = self.notify_q.len() < NOTIFY_MAX_FRAMES
                    && self.notify_queued_bytes.saturating_add(frame.len()) <= NOTIFY_MAX_BYTES;
                if fits_after {
                    self.notify_queued_bytes = self.notify_queued_bytes.saturating_add(frame.len());
                    if front {
                        self.notify_q.push_front(frame);
                    } else {
                        self.notify_q.push_back(frame);
                    }
                    return Ok(());
                }
                let now_ms = stats::now_ms();
                if now_ms.saturating_sub(self.last_status_drop_log_ms) >= STATUS_DROP_LOG_PERIOD_MS
                {
                    self.last_status_drop_log_ms = now_ms;
                    warn!("gateway notify queue full; dropping low-priority SnifferStatus");
                }
                Ok(())
            }
            Some(
                wardriver_packlink::MsgType::ObservationBulk
                | wardriver_packlink::MsgType::EventNotify,
            ) => psram_spool::append(&frame),
            kind => {
                warn!("gateway notify queue full for control frame type={kind:?}; spooling");
                psram_spool::append(&frame)
            }
        }
    }

    fn pop_notify_front(&mut self) -> Option<Vec<u8>> {
        let frame = self.notify_q.pop_front()?;
        self.notify_queued_bytes = self.notify_queued_bytes.saturating_sub(frame.len());
        Some(frame)
    }

    fn flush_notify_queue(&mut self) {
        if !self.phone_connected() {
            // The transport clears its wire queue on disconnect, so restart any
            // partial spool replay from retained PSRAM bytes on the next session.
            self.spool_replay = None;
            softap_transport::reset_spool_replay_snapshot();
            return;
        }
        let mut wire_backpressured = false;
        for _ in 0..NOTIFY_FLUSH_MAX_PER_TICK {
            let Some(frame) = self.pop_notify_front() else {
                break;
            };
            match softap_transport::try_send_frame(frame) {
                Ok(()) => task_wdt::reset_if_subscribed(),
                Err(frame) => {
                    self.notify_queued_bytes = self.notify_queued_bytes.saturating_add(frame.len());
                    self.notify_q.push_front(frame);
                    wire_backpressured = true;
                    break;
                }
            }
        }
        if !wire_backpressured && self.notify_q.is_empty() && psram_spool::used_bytes() > 0 {
            self.replay_spool();
        }
    }

    fn replay_spool(&mut self) {
        if self
            .spool_replay
            .as_ref()
            .is_some_and(|cursor| psram_spool::replay_snapshot().1 != cursor.tail_epoch)
        {
            warn!("gateway spool tail advanced during replay; restarting snapshot");
            self.spool_replay = None;
            softap_transport::reset_spool_replay_snapshot();
        }
        if self.spool_replay.is_none() {
            let (total_bytes, tail_epoch) = psram_spool::replay_snapshot();
            if total_bytes == 0 {
                return;
            }
            self.replay_nonce = self.replay_nonce.wrapping_add(1).max(1);
            let snapshot_nonce = self.replay_nonce;
            let session_generation = softap_transport::session_generation();
            softap_transport::begin_spool_replay_snapshot(session_generation, snapshot_nonce);
            self.spool_replay = Some(SpoolReplayCursor {
                total_bytes,
                tail_epoch,
                snapshot_nonce,
                offset: 0,
                chunk_index: 0,
                chunk_total: total_bytes.div_ceil(SPOOL_CHUNK as u32),
                session_generation,
            });
        }

        for _ in 0..SPOOL_REPLAY_MAX_PER_TICK {
            if self
                .spool_replay
                .as_ref()
                .is_some_and(|cursor| cursor.offset >= cursor.total_bytes)
            {
                self.finish_spool_replay_if_complete();
                break;
            }
            let Some(cursor) = self.spool_replay.as_mut() else {
                break;
            };
            let remaining = cursor.total_bytes.saturating_sub(cursor.offset) as usize;
            let Ok(bytes) = psram_spool::read_chunk(cursor.offset, SPOOL_CHUNK.min(remaining))
            else {
                break;
            };
            if bytes.is_empty() {
                break;
            }
            let payload_len = bytes.len() as u32;
            let chunk = SpoolReplayV1 {
                spool_offset: cursor.offset,
                chunk_total: cursor.chunk_total,
                chunk_index: cursor.chunk_index,
                payload: bytes,
            };
            let frame = pack_spool_replay(&chunk, cursor.snapshot_nonce);
            if softap_transport::try_send_frame(frame).is_err() {
                break;
            }
            cursor.offset = cursor.offset.saturating_add(payload_len);
            cursor.chunk_index = cursor.chunk_index.saturating_add(1);
            task_wdt::reset_if_subscribed();
        }

        // Re-check completion after the loop so a spool that only finished confirming
        // while the last chunk was in flight is cleared this tick (the in-loop attempt
        // above already `break`s on completion, so this is a no-op in the common case).
        self.finish_spool_replay_if_complete();
    }

    /// If the active spool replay is fully written and confirmed by the phone, clear the
    /// replayed prefix from PSRAM. Safe to call when no replay is active or it is
    /// incomplete: it is then a no-op.
    fn finish_spool_replay_if_complete(&mut self) {
        let Some(cursor) = self.spool_replay.as_ref() else {
            return;
        };
        if cursor.offset < cursor.total_bytes {
            return;
        }
        if !softap_transport::phone_connected() {
            return;
        }
        if softap_transport::session_generation() != cursor.session_generation {
            return;
        }
        if softap_transport::spool_replay_written_end(
            cursor.session_generation,
            cursor.snapshot_nonce,
        ) < cursor.total_bytes
        {
            return;
        }
        if !softap_transport::spool_replay_confirmed_for(
            cursor.session_generation,
            cursor.snapshot_nonce,
        ) {
            return;
        }
        match psram_spool::clear_replayed_prefix(cursor.total_bytes, cursor.tail_epoch) {
            Ok(true) => {
                self.spool_replay = None;
                softap_transport::reset_spool_replay_snapshot();
            }
            Ok(false) => {
                warn!("gateway spool changed during replay; restarting snapshot");
                self.spool_replay = None;
                softap_transport::reset_spool_replay_snapshot();
            }
            Err(e) => warn!("gateway spool clear replayed prefix: {e:#}"),
        }
    }
}

fn softap_frame_type(frame: &[u8]) -> Option<wardriver_packlink::MsgType> {
    wardriver_packlink::unpack_frame(frame)
        .ok()
        .map(|decoded| decoded.msg_type)
}

fn is_detection_alert_frame(frame: &RxFrame) -> bool {
    let p = frame.payload();
    p.len() >= 2 && p[0] == PROTO_VERSION && p[1] == FrameType::DetectionAlert as u8
}

fn sniffer_id_from_event_payload(kind: u8, payload: &[u8]) -> u32 {
    use wardriver_packlink::event_kind;
    match kind {
        event_kind::DETECTION_WIFI => unpack_detection_alert(payload)
            .map(|a| a.sniffer_id)
            .unwrap_or(0),
        event_kind::DETECTION_BLE => unpack_detection_ble_alert(payload)
            .map(|a| a.sniffer_id)
            .unwrap_or(0),
        event_kind::SSID_WATCH => unpack_ssid_watch_alert(payload)
            .map(|a| a.sniffer_id)
            .unwrap_or(0),
        event_kind::REMOTE_ID => unpack_remote_id_alert(payload)
            .map(|a| a.sniffer_id)
            .unwrap_or(0),
        _ => 0,
    }
}

/// Signal the bridge thread to flush queued PackLink frames / spool replay.
pub fn signal_flush(tick: &SyncSender<()>) {
    let _ = tick.try_send(());
}
