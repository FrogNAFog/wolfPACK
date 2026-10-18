//! PackLink over gateway SoftAP: plaintext TCP session and TOFU handshake.
//!
//! All TCP I/O runs on the PackLink session pthread only. Producers enqueue outbound
//! frames; the main loop processes heavy inbound work via [`poll_inbound_work`].

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::Duration;

use crate::gateway_adoption::{self, AdoptProgress, AdoptReason};
use anyhow::{anyhow, Result};
use log::{info, warn};
use wardriver_fleet_realtime::peer_pool;
use wardriver_packlink::{
    adopt_progress, adopt_reason, channel_plan_sync_ack_error, channel_plan_sync_ack_result,
    default_gateway_info, pack_channel_plan_sync_ack, pack_fleet_unbind_notify, pack_gateway_info,
    pack_policy_sync_ack, pack_sniffer_status_batch, pack_time_sync, policy_sync_ack_error,
    policy_sync_ack_result, unpack_channel_plan_sync, unpack_phone_hello, unpack_policy_sync,
    unpack_softap_config, unpack_time_sync, ChannelPlanSyncAckV1, GatewayInfoV1, MsgType,
    PolicySyncAckV1, SnifferStatusAdoptExtV1, SnifferStatusV1, TimeSyncV1, MAGIC,
    PACKLINK_TCP_PORT, PACKLINK_VERSION,
};
use wardriver_proto::PROTO_VERSION;
use wardriver_shared::spiram_thread;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::capture_job;
use crate::channel_plan_downlink;
use crate::gateway_softap;
use crate::packlink_nvs_defer::{
    PacklinkHandshakeReply, PacklinkNvsDeferQueue, PACKLINK_NVS_DEFER_WAIT_MS,
};
use crate::packlink_trust;
use crate::policy_downlink::{self, GatewayFleetState};
use crate::psram_spool;

static OUTBOUND_Q: OnceLock<Mutex<OutboundQueue>> = OnceLock::new();
static INBOUND_Q: OnceLock<Mutex<VecDeque<InboundWork>>> = OnceLock::new();
static FLEET: OnceLock<Arc<GatewayFleetState>> = OnceLock::new();
static NVS_DEFER: OnceLock<PacklinkNvsDeferQueue> = OnceLock::new();
static CONNECT_FLUSH_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

static PHONE_CONNECTED: AtomicBool = AtomicBool::new(false);
static SESSION_ABORT_REQUESTED: AtomicBool = AtomicBool::new(false);
static SESSION_GENERATION: AtomicU32 = AtomicU32::new(0);
static NOTIFY_SEQ: AtomicU32 = AtomicU32::new(0);
static LAST_SNIFFER_STATUS_MS: AtomicU32 = AtomicU32::new(0);
static SPOOL_REPLAY_WRITTEN_END: AtomicU32 = AtomicU32::new(0);
static SPOOL_REPLAY_SNAPSHOT_NONCE: AtomicU32 = AtomicU32::new(0);
static SPOOL_REPLAY_SNAPSHOT_SESSION: AtomicU32 = AtomicU32::new(0);
static SPOOL_REPLAY_CONFIRMED_SESSION: AtomicU32 = AtomicU32::new(0);
static SPOOL_REPLAY_CONFIRMED_NONCE: AtomicU32 = AtomicU32::new(0);

const SNIFFER_STATUS_PERIOD_MS: u64 = 2_000;
const HANDSHAKE_IO_TIMEOUT_MS: u64 = 15_000;
const SESSION_IO_TIMEOUT_MS: u64 = 1_000;
const QUEUE_LOCK_TIMEOUT_MS: u64 = 25;
const OUTBOUND_MAX_FRAMES: usize = 64;
const OUTBOUND_MAX_BYTES: usize = 96 * 1024;
const OUTBOUND_BULK_MAX_FRAMES: usize = 60;
const OUTBOUND_BULK_MAX_BYTES: usize = 88 * 1024;
const MANDATORY_MAX_FRAMES: usize = 16;
const MANDATORY_MAX_BYTES: usize = 16 * 1024;
const INBOUND_MAX_WORK: usize = 32;
const SESSION_DRAIN_MAX_FRAMES: usize = 8;
const SLOW_WRITE_MS: u64 = 200;
const ENQUEUE_RETRY_ATTEMPTS: usize = 3;
pub(crate) const SPOOL_REPLAY_CONFIRM_MARKER: u32 = 0x5350_4F4C;

enum InboundWork {
    PolicySync(Vec<u8>, u32),
    ChannelPlanSync(Vec<u8>, u32),
    ControlCmd(Vec<u8>),
    CaptureCmd(Vec<u8>),
    TimeSync(Vec<u8>, u32),
    SoftApConfig(Vec<u8>),
    TrustForget,
}

#[derive(Default)]
struct OutboundQueue {
    mandatory_frames: VecDeque<Vec<u8>>,
    mandatory_bytes: usize,
    frames: VecDeque<Vec<u8>>,
    queued_bytes: usize,
}

impl OutboundQueue {
    fn pop_front(&mut self) -> Option<Vec<u8>> {
        if let Some(frame) = self.mandatory_frames.pop_front() {
            self.mandatory_bytes = self.mandatory_bytes.saturating_sub(frame.len());
            return Some(frame);
        }
        let frame = self.frames.pop_front()?;
        self.queued_bytes = self.queued_bytes.saturating_sub(frame.len());
        Some(frame)
    }

    fn can_admit(&self, frame_len: usize, bulk_or_status: bool) -> bool {
        let (max_frames, max_bytes) = if bulk_or_status {
            (OUTBOUND_BULK_MAX_FRAMES, OUTBOUND_BULK_MAX_BYTES)
        } else {
            (OUTBOUND_MAX_FRAMES, OUTBOUND_MAX_BYTES)
        };
        self.frames.len() < max_frames && self.queued_bytes.saturating_add(frame_len) <= max_bytes
    }

    fn push_back(&mut self, frame: Vec<u8>) {
        self.queued_bytes = self.queued_bytes.saturating_add(frame.len());
        self.frames.push_back(frame);
    }

    fn push_front(&mut self, frame: Vec<u8>) {
        self.queued_bytes = self.queued_bytes.saturating_add(frame.len());
        self.frames.push_front(frame);
    }

    fn push_mandatory(&mut self, frame: Vec<u8>) -> std::result::Result<(), Vec<u8>> {
        if self.mandatory_frames.len() >= MANDATORY_MAX_FRAMES
            || self.mandatory_bytes.saturating_add(frame.len()) > MANDATORY_MAX_BYTES
        {
            return Err(frame);
        }
        self.mandatory_bytes = self.mandatory_bytes.saturating_add(frame.len());
        self.mandatory_frames.push_back(frame);
        Ok(())
    }
}

fn outbound_queue() -> &'static Mutex<OutboundQueue> {
    OUTBOUND_Q.get_or_init(|| Mutex::new(OutboundQueue::default()))
}

fn try_lock_wdt<T>(mutex: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    let deadline = stats::now_ms().saturating_add(QUEUE_LOCK_TIMEOUT_MS);
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(_)) => return None,
            Err(TryLockError::WouldBlock) if stats::now_ms() < deadline => {
                task_wdt::sleep_feeding_wdt(Duration::from_millis(1));
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

fn frame_msg_type(frame: &[u8]) -> Option<MsgType> {
    wardriver_packlink::unpack_frame(frame)
        .ok()
        .map(|decoded| decoded.msg_type)
}

fn is_priority_frame(frame: &[u8]) -> bool {
    matches!(
        frame_msg_type(frame),
        Some(
            MsgType::GatewayInfo
                | MsgType::TrustOk
                | MsgType::TrustReject
                | MsgType::PolicySyncAck
                | MsgType::EventNotify
                | MsgType::CaptureStatus
                | MsgType::FleetUnbindNotify
        )
    )
}

fn is_bulk_or_status(frame: &[u8]) -> bool {
    matches!(
        frame_msg_type(frame),
        Some(MsgType::ObservationBulk | MsgType::SpoolReplay | MsgType::SnifferStatus)
    )
}

fn is_mandatory_frame(frame: &[u8]) -> bool {
    matches!(
        frame_msg_type(frame),
        Some(MsgType::PolicySyncAck | MsgType::TimeSync)
    )
}

fn replace_queued_sniffer_status(queue: &mut OutboundQueue) {
    if let Some(idx) = queue
        .frames
        .iter()
        .rposition(|f| frame_msg_type(f) == Some(MsgType::SnifferStatus))
    {
        if let Some(old) = queue.frames.remove(idx) {
            queue.queued_bytes = queue.queued_bytes.saturating_sub(old.len());
        }
    }
}

fn enqueue_outbound(frame: Vec<u8>, front: bool) -> std::result::Result<(), Vec<u8>> {
    if frame.len() > OUTBOUND_MAX_BYTES {
        return Err(frame);
    }
    let priority = is_priority_frame(&frame);
    let pressure = is_bulk_or_status(&frame);
    let is_status = frame_msg_type(&frame) == Some(MsgType::SnifferStatus);
    let Some(mut queue) = try_lock_wdt(outbound_queue()) else {
        return Err(frame);
    };
    if !queue.can_admit(frame.len(), if priority { false } else { pressure }) {
        if is_status {
            replace_queued_sniffer_status(&mut queue);
        }
        if !queue.can_admit(frame.len(), if priority { false } else { pressure }) {
            return Err(frame);
        }
    }
    if front || priority {
        queue.push_front(frame);
    } else {
        queue.push_back(frame);
    }
    Ok(())
}

fn send_mandatory_frame(mut frame: Vec<u8>) -> bool {
    for _ in 0..ENQUEUE_RETRY_ATTEMPTS {
        let Some(mut queue) = try_lock_wdt(outbound_queue()) else {
            task_wdt::sleep_feeding_wdt(Duration::from_millis(1));
            continue;
        };
        match queue.push_mandatory(frame) {
            Ok(()) => return true,
            Err(returned) => frame = returned,
        }
        drop(queue);
        task_wdt::sleep_feeding_wdt(Duration::from_millis(1));
    }
    warn!(
        "packlink softap: mandatory outbound enqueue failed type={:?} bytes={}",
        frame_msg_type(&frame),
        frame.len()
    );
    SESSION_ABORT_REQUESTED.store(true, Ordering::Release);
    false
}

pub fn register_fleet(
    state: Arc<GatewayFleetState>,
    on_connect_flush: impl Fn() + Send + Sync + 'static,
) {
    let _ = FLEET.set(state);
    let _ = CONNECT_FLUSH_HOOK.set(Box::new(on_connect_flush));
    let _ = outbound_queue();
    let _ = inbound_queue();
}

/// Bind the PackLink NVS defer queue (call once from gateway boot before TCP accept).
pub fn register_nvs_defer(queue: PacklinkNvsDeferQueue) {
    let _ = NVS_DEFER.set(queue);
}

/// Start the PackLink TCP accept thread on the gateway SoftAP address.
pub fn init_softap_transport() -> Result<()> {
    spawn_tcp_server_thread();
    info!("packlink softap: tcp listening on 192.168.71.1:{PACKLINK_TCP_PORT}");
    Ok(())
}

pub fn send_frame(mut bytes: Vec<u8>) -> bool {
    for _ in 0..ENQUEUE_RETRY_ATTEMPTS {
        match enqueue_outbound(bytes, false) {
            Ok(()) => return true,
            Err(frame) => {
                bytes = frame;
                task_wdt::sleep_feeding_wdt(Duration::from_millis(1));
            }
        }
    }
    warn!(
        "packlink softap: outbound enqueue rejected type={:?} bytes={}",
        frame_msg_type(&bytes),
        bytes.len()
    );
    false
}

pub fn try_send_frame(bytes: Vec<u8>) -> std::result::Result<(), Vec<u8>> {
    enqueue_outbound(bytes, false)
}

/// Process PackLink inbound work queued by the TCP session thread (main-loop only).
pub fn poll_inbound_work() {
    let mut pending = Vec::new();
    if let Some(mut queue) = try_lock_wdt(inbound_queue()) {
        let queued = queue.len();
        let mut took_policy = false;
        let mut took_channel_plan = false;
        for _ in 0..queued {
            let Some(work) = queue.pop_front() else {
                break;
            };
            if matches!(work, InboundWork::PolicySync(_, _)) {
                if took_policy {
                    queue.push_back(work);
                    continue;
                }
                took_policy = true;
            }
            if matches!(work, InboundWork::ChannelPlanSync(_, _)) {
                if took_channel_plan {
                    queue.push_back(work);
                    continue;
                }
                took_channel_plan = true;
            }
            pending.push(work);
        }
    }
    for work in pending {
        if let Err(e) = process_inbound_work(work) {
            warn!("packlink softap: inbound work: {e:#}");
        }
    }
}

pub fn maybe_notify_sniffer_status() {
    if !phone_connected() {
        return;
    }
    let now = stats::now_ms();
    let last = u64::from(LAST_SNIFFER_STATUS_MS.load(Ordering::Relaxed));
    if now.saturating_sub(last) < SNIFFER_STATUS_PERIOD_MS {
        return;
    }
    LAST_SNIFFER_STATUS_MS.store(now.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
    let rows = build_sniffer_status_rows();
    if rows.is_empty() {
        return;
    }
    let gateway_mono_ms = now.min(u64::from(u32::MAX)) as u32;
    let seq = NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed);
    let _ = send_frame(pack_sniffer_status_batch(&rows, gateway_mono_ms, seq));
}

/// Tell the connected phone to clear its Room fleet roster after a physical unbind.
pub fn notify_fleet_unbind_if_connected() {
    if !phone_connected() {
        return;
    }
    let seq = NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed);
    if !send_frame(pack_fleet_unbind_notify(seq)) {
        warn!("packlink softap: fleet unbind notify enqueue failed");
    }
}

#[must_use]
pub fn phone_connected() -> bool {
    PHONE_CONNECTED.load(Ordering::Acquire)
}

#[must_use]
pub(crate) fn spool_replay_written_end(session_generation: u32, snapshot_nonce: u32) -> u32 {
    if SPOOL_REPLAY_SNAPSHOT_SESSION.load(Ordering::Acquire) != session_generation
        || SPOOL_REPLAY_SNAPSHOT_NONCE.load(Ordering::Acquire) != snapshot_nonce
    {
        return 0;
    }
    SPOOL_REPLAY_WRITTEN_END.load(Ordering::Acquire)
}

#[must_use]
pub(crate) fn session_generation() -> u32 {
    SESSION_GENERATION.load(Ordering::Acquire)
}

#[must_use]
pub(crate) fn spool_replay_confirmed_for(session_generation: u32, snapshot_nonce: u32) -> bool {
    session_generation != 0
        && snapshot_nonce != 0
        && SPOOL_REPLAY_CONFIRMED_SESSION.load(Ordering::Acquire) == session_generation
        && SPOOL_REPLAY_CONFIRMED_NONCE.load(Ordering::Acquire) == snapshot_nonce
}

pub(crate) fn begin_spool_replay_snapshot(session_generation: u32, snapshot_nonce: u32) {
    SPOOL_REPLAY_SNAPSHOT_SESSION.store(session_generation, Ordering::Release);
    SPOOL_REPLAY_SNAPSHOT_NONCE.store(snapshot_nonce, Ordering::Release);
    SPOOL_REPLAY_WRITTEN_END.store(0, Ordering::Release);
    SPOOL_REPLAY_CONFIRMED_SESSION.store(0, Ordering::Release);
    SPOOL_REPLAY_CONFIRMED_NONCE.store(0, Ordering::Release);
}

pub(crate) fn reset_spool_replay_snapshot() {
    SPOOL_REPLAY_SNAPSHOT_SESSION.store(0, Ordering::Release);
    SPOOL_REPLAY_SNAPSHOT_NONCE.store(0, Ordering::Release);
    SPOOL_REPLAY_WRITTEN_END.store(0, Ordering::Release);
    SPOOL_REPLAY_CONFIRMED_SESSION.store(0, Ordering::Release);
    SPOOL_REPLAY_CONFIRMED_NONCE.store(0, Ordering::Release);
}

pub(crate) fn set_phone_connected(connected: bool) {
    PHONE_CONNECTED.store(connected, Ordering::Release);
    if connected {
        if let Some(f) = fleet() {
            if crate::policy_downlink::policy_mirror_has_fleet_config(f) {
                f.policy_applied.store(true, Ordering::Release);
            }
        }
    } else {
        reset_spool_replay_snapshot();
    }
}

fn fleet() -> Option<&'static Arc<GatewayFleetState>> {
    FLEET.get()
}

fn build_gateway_info() -> GatewayInfoV1 {
    let mut info = default_gateway_info();
    info.packlink_version = PACKLINK_VERSION;
    info.proto_version = PROTO_VERSION;
    info.spool_bytes_total = psram_spool::capacity_bytes();
    info.spool_bytes_used = psram_spool::used_bytes();
    info.peer_slots_free = peer_pool::peer_slots_free_estimate();
    info.peer_join_waiters = peer_pool::join_queue_waiter_count();
    if let Some(f) = fleet() {
        info.gateway_boot_id = f.control_boot_id.load(Ordering::Relaxed);
    }
    if let Ok(mac) = gateway_softap::read_softap_mac() {
        info.gateway_mac = mac;
    }
    info.capabilities = channel_plan_downlink::gateway_capabilities();
    let _ = psram_spool::fill_percent();
    info
}

#[must_use]
pub fn gateway_info_bytes() -> Vec<u8> {
    pack_gateway_info(&build_gateway_info())
}

pub fn on_policy_sync_write(data: &[u8]) -> Result<wardriver_packlink::PolicySyncV1> {
    let frame = wardriver_packlink::unpack_frame(data)?;
    unpack_policy_sync(frame.payload)
}

pub fn on_control_cmd_write(data: &[u8]) -> Result<Vec<u8>> {
    let frame = wardriver_packlink::unpack_frame(data)?;
    Ok(frame.payload.to_vec())
}

pub fn on_time_sync_write(data: &[u8]) -> Result<TimeSyncV1> {
    let frame = wardriver_packlink::unpack_frame(data)?;
    unpack_time_sync(frame.payload)
}

#[must_use]
pub fn build_sniffer_status_rows() -> Vec<SnifferStatusV1> {
    let now_ms = stats::now_ms();
    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    stats::for_each_sniffer_slot(|s| {
        seen_ids.insert(s.sniffer_id);
        let heap_avail_pct =
            stats::heap_avail_pct_from(s.free_heap_bytes, s.total_heap_bytes).unwrap_or(0);
        let (progress, reason) = gateway_adoption::adopt_status(s.sniffer_id, now_ms);
        let adoption_state = adoption_state_from_progress(progress, s.adoption_state);
        let adopt_ext = adopt_ext_from_fsm(progress, reason);
        rows.push(SnifferStatusV1 {
            sniffer_id: s.sniffer_id,
            mac: s.mac,
            last_seen_ms: s.last_seen_ms.min(u64::from(u32::MAX)) as u32,
            state: s.state,
            role: s.role,
            cpu_load_pct: s.cpu_load_pct,
            heap_avail_pct,
            fleet_slot: s.fleet_slot,
            reboot_count: s.reboot_count.min(u16::MAX as u32) as u16,
            adoption_state,
            plan_ext: channel_plan_downlink::plan_ext_for_slot(
                s.five_ghz_capable,
                s.assigned_band,
                s.pinned_channel,
                s.assigned_chan_count,
                &s.assigned_channels,
            ),
            adopt_ext,
        });
    });
    for (sniffer_id, mac, fleet_slot, progress, reason) in
        gateway_adoption::pending_status_rows(now_ms)
    {
        if seen_ids.contains(&sniffer_id) {
            continue;
        }
        let adoption_state =
            adoption_state_from_progress(progress, wardriver_packlink::adoption_state::PENDING);
        let adopt_ext = adopt_ext_from_fsm(progress, reason);
        rows.push(SnifferStatusV1 {
            sniffer_id,
            mac,
            last_seen_ms: 0,
            state: 0,
            role: 0,
            cpu_load_pct: 0,
            heap_avail_pct: 0,
            fleet_slot,
            reboot_count: 0,
            adoption_state,
            plan_ext: None,
            adopt_ext,
        });
    }
    rows
}

fn adoption_state_from_progress(progress: AdoptProgress, slot_state: u8) -> u8 {
    if progress == AdoptProgress::Confirmed {
        return wardriver_packlink::adoption_state::ADOPTED;
    }
    if progress != AdoptProgress::None {
        return wardriver_packlink::adoption_state::PENDING;
    }
    slot_state
}

fn adopt_ext_from_fsm(
    progress: AdoptProgress,
    reason: AdoptReason,
) -> Option<SnifferStatusAdoptExtV1> {
    if progress == AdoptProgress::None && reason == AdoptReason::None {
        return None;
    }
    Some(SnifferStatusAdoptExtV1 {
        adopt_progress: adopt_progress_wire(progress),
        adopt_reason: adopt_reason_wire(reason),
    })
}

fn adopt_progress_wire(progress: AdoptProgress) -> u8 {
    match progress {
        AdoptProgress::None => adopt_progress::NONE,
        AdoptProgress::WaitingPolicy => adopt_progress::WAITING_POLICY,
        AdoptProgress::RetryingWelcome => adopt_progress::RETRYING_WELCOME,
        AdoptProgress::Confirmed => adopt_progress::CONFIRMED,
        AdoptProgress::TimedOutHelloLive => adopt_progress::TIMED_OUT_HELLO_LIVE,
        AdoptProgress::TimedOutNoHello => adopt_progress::TIMED_OUT_NO_HELLO,
    }
}

fn adopt_reason_wire(reason: AdoptReason) -> u8 {
    match reason {
        AdoptReason::None => adopt_reason::NONE,
        AdoptReason::SendFailed => adopt_reason::SEND_FAILED,
        AdoptReason::PossibleExistingBinding => adopt_reason::POSSIBLE_EXISTING_BINDING,
        AdoptReason::RosterRemoved => adopt_reason::ROSTER_REMOVED,
    }
}

fn time_sync_reply(ts: &TimeSyncV1, seq: u32) -> Vec<u8> {
    let mut stamped = *ts;
    stamped.gateway_mono_ms = stats::now_ms().min(u64::from(u32::MAX)) as u32;
    pack_time_sync(&stamped, seq)
}

fn send_policy_sync_ack(policy_seq: u32, result: u8, error_code: u8) -> bool {
    let ack = PolicySyncAckV1 {
        policy_seq,
        result,
        error_code,
    };
    let seq = NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed);
    send_mandatory_frame(pack_policy_sync_ack(&ack, seq))
}

fn send_channel_plan_sync_ack(ack: &ChannelPlanSyncAckV1) -> bool {
    let seq = NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed);
    send_mandatory_frame(pack_channel_plan_sync_ack(ack, seq))
}

fn clear_outbound_queue() -> bool {
    if let Some(mut queue) = try_lock_wdt(outbound_queue()) {
        queue.frames.clear();
        queue.queued_bytes = 0;
        queue.mandatory_frames.clear();
        queue.mandatory_bytes = 0;
        true
    } else {
        warn!("packlink softap: outbound queue reset unavailable");
        false
    }
}

/// Drop every queued `PolicySync`/`ChannelPlanSync` work item carrying `new_generation`
/// (a newer submission of the same generation), collecting the seqs that must be
/// rejected. Order-preserving: called after the generation is extracted, exactly where
/// the inline `queue.retain` blocks previously ran.
fn supersede_generation(
    queue: &mut VecDeque<InboundWork>,
    variant: InboundVariant,
    new_generation: u32,
    superseded_seqs: &mut Vec<u32>,
) {
    queue.retain(|pending| match (variant, pending) {
        (InboundVariant::Policy, InboundWork::PolicySync(data, generation))
        | (InboundVariant::Plan, InboundWork::ChannelPlanSync(data, generation)) => {
            if *generation == new_generation {
                if let Ok(frame) = wardriver_packlink::unpack_frame(data) {
                    superseded_seqs.push(frame.seq);
                }
            }
            false
        }
        _ => true,
    });
}

#[derive(Clone, Copy)]
enum InboundVariant {
    Policy,
    Plan,
}

fn enqueue_inbound(work: InboundWork) -> bool {
    let mut superseded_policy_seqs = Vec::new();
    let mut superseded_plan_seqs = Vec::new();
    let (rejected_policy_seq, rejected_plan_seq) =
        if let Some(mut queue) = try_lock_wdt(inbound_queue()) {
            if matches!(work, InboundWork::PolicySync(_, _)) {
                let new_generation = match &work {
                    InboundWork::PolicySync(_, generation) => *generation,
                    _ => 0,
                };
                supersede_generation(
                    &mut queue,
                    InboundVariant::Policy,
                    new_generation,
                    &mut superseded_policy_seqs,
                );
            }
            if matches!(work, InboundWork::ChannelPlanSync(_, _)) {
                let new_generation = match &work {
                    InboundWork::ChannelPlanSync(_, generation) => *generation,
                    _ => 0,
                };
                supersede_generation(
                    &mut queue,
                    InboundVariant::Plan,
                    new_generation,
                    &mut superseded_plan_seqs,
                );
            }
            if queue.len() < INBOUND_MAX_WORK {
                queue.push_back(work);
                (None, None)
            } else {
                match &work {
                    InboundWork::PolicySync(data, _) => (
                        wardriver_packlink::unpack_frame(data)
                            .ok()
                            .map(|frame| frame.seq),
                        None,
                    ),
                    InboundWork::ChannelPlanSync(data, _) => (
                        None,
                        wardriver_packlink::unpack_frame(data)
                            .ok()
                            .map(|frame| frame.seq),
                    ),
                    _ => {
                        warn!("packlink softap: bounded inbound queue full; control work rejected");
                        return false;
                    }
                }
            }
        } else {
            match &work {
                InboundWork::PolicySync(data, _) => (
                    wardriver_packlink::unpack_frame(data)
                        .ok()
                        .map(|frame| frame.seq),
                    None,
                ),
                InboundWork::ChannelPlanSync(data, _) => (
                    None,
                    wardriver_packlink::unpack_frame(data)
                        .ok()
                        .map(|frame| frame.seq),
                ),
                _ => {
                    warn!("packlink softap: inbound queue lock timeout; control work rejected");
                    return false;
                }
            }
        };
    for policy_seq in superseded_policy_seqs {
        if !send_policy_sync_ack(
            policy_seq,
            policy_sync_ack_result::REJECTED,
            policy_sync_ack_error::APPLY,
        ) {
            return false;
        }
    }
    for plan_seq in superseded_plan_seqs {
        let ack = ChannelPlanSyncAckV1 {
            plan_seq,
            revision: 0,
            result: channel_plan_sync_ack_result::REJECTED,
            error_code: channel_plan_sync_ack_error::APPLY,
            hop_cycle_ms: 0,
            unassigned_count: 0,
        };
        if !send_channel_plan_sync_ack(&ack) {
            return false;
        }
    }
    if let Some(policy_seq) = rejected_policy_seq {
        return send_policy_sync_ack(
            policy_seq,
            policy_sync_ack_result::REJECTED,
            policy_sync_ack_error::APPLY,
        );
    }
    if let Some(plan_seq) = rejected_plan_seq {
        let ack = ChannelPlanSyncAckV1 {
            plan_seq,
            revision: 0,
            result: channel_plan_sync_ack_result::REJECTED,
            error_code: channel_plan_sync_ack_error::APPLY,
            hop_cycle_ms: 0,
            unassigned_count: 0,
        };
        return send_channel_plan_sync_ack(&ack);
    }
    true
}

fn inbound_queue() -> &'static Mutex<VecDeque<InboundWork>> {
    INBOUND_Q.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn process_inbound_work(work: InboundWork) -> Result<()> {
    match work {
        InboundWork::PolicySync(data, inbound_session_generation) => {
            if !phone_connected() || session_generation() != inbound_session_generation {
                return Ok(());
            }
            let frame = wardriver_packlink::unpack_frame(&data)?;
            let policy_seq = frame.seq;
            match unpack_policy_sync(frame.payload) {
                Ok(policy) => {
                    let Some(fleet) = fleet() else {
                        if !send_policy_sync_ack(
                            policy_seq,
                            policy_sync_ack_result::REJECTED,
                            policy_sync_ack_error::APPLY,
                        ) {
                            return Err(anyhow!("PolicySync rejection delivery unavailable"));
                        }
                        return Ok(());
                    };
                    match policy_downlink::apply_policy_sync(fleet, policy) {
                        Ok(()) => {
                            if !send_policy_sync_ack(
                                policy_seq,
                                policy_sync_ack_result::OK,
                                policy_sync_ack_error::NONE,
                            ) {
                                return Err(anyhow!("PolicySync ack delivery unavailable"));
                            }
                        }
                        Err(e) => {
                            warn!("packlink softap: apply_policy_sync: {e:#}");
                            if !send_policy_sync_ack(
                                policy_seq,
                                policy_sync_ack_result::REJECTED,
                                policy_sync_ack_error::APPLY,
                            ) {
                                return Err(anyhow!("PolicySync rejection delivery unavailable"));
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("packlink softap: unpack_policy_sync: {e:#}");
                    if !send_policy_sync_ack(
                        policy_seq,
                        policy_sync_ack_result::REJECTED,
                        policy_sync_ack_error::PARSE,
                    ) {
                        return Err(anyhow!("PolicySync parse rejection delivery unavailable"));
                    }
                }
            }
        }
        InboundWork::ChannelPlanSync(data, inbound_session_generation) => {
            if !phone_connected() || session_generation() != inbound_session_generation {
                return Ok(());
            }
            let frame = wardriver_packlink::unpack_frame(&data)?;
            let plan_seq = frame.seq;
            match unpack_channel_plan_sync(frame.payload) {
                Ok(plan) => {
                    let Some(fleet) = fleet() else {
                        let ack = ChannelPlanSyncAckV1 {
                            plan_seq,
                            revision: plan.revision,
                            result: channel_plan_sync_ack_result::REJECTED,
                            error_code: channel_plan_sync_ack_error::APPLY,
                            hop_cycle_ms: 0,
                            unassigned_count: 0,
                        };
                        if !send_channel_plan_sync_ack(&ack) {
                            return Err(anyhow!("ChannelPlanSync rejection delivery unavailable"));
                        }
                        return Ok(());
                    };
                    let ack = channel_plan_downlink::apply_channel_plan_sync(fleet, plan, plan_seq);
                    if !send_channel_plan_sync_ack(&ack) {
                        return Err(anyhow!("ChannelPlanSync ack delivery unavailable"));
                    }
                }
                Err(e) => {
                    warn!("packlink softap: unpack_channel_plan_sync: {e:#}");
                    let ack = ChannelPlanSyncAckV1 {
                        plan_seq,
                        revision: 0,
                        result: channel_plan_sync_ack_result::REJECTED,
                        error_code: channel_plan_sync_ack_error::PARSE,
                        hop_cycle_ms: 0,
                        unassigned_count: 0,
                    };
                    if !send_channel_plan_sync_ack(&ack) {
                        return Err(anyhow!(
                            "ChannelPlanSync parse rejection delivery unavailable"
                        ));
                    }
                }
            }
        }
        InboundWork::ControlCmd(data) => {
            let payload = on_control_cmd_write(&data)?;
            if let Err(e) = wardriver_shared::espnow::add_broadcast_peer(
                wardriver_fleet_realtime::CONTROL_AP_CHANNEL,
                wardriver_fleet_realtime::ESPNOW_PEER_IF,
            ) {
                warn!("packlink control_cmd add_broadcast_peer: {e:#}");
            }
            if let Err(e) =
                wardriver_shared::espnow::send(wardriver_shared::espnow::BROADCAST_MAC, &payload)
            {
                warn!("packlink control_cmd espnow send: {e:#}");
            }
        }
        InboundWork::CaptureCmd(data) => {
            if let Some(fleet) = fleet() {
                capture_job::try_handle_control_cmd_write(&data, fleet)?;
            }
        }
        InboundWork::TimeSync(data, inbound_session_generation) => {
            if !phone_connected() || session_generation() != inbound_session_generation {
                return Ok(());
            }
            let ts = on_time_sync_write(&data)?;
            if ts.gateway_mono_ms == SPOOL_REPLAY_CONFIRM_MARKER
                && ts.time_set_seq != 0
                && SPOOL_REPLAY_SNAPSHOT_SESSION.load(Ordering::Acquire)
                    == inbound_session_generation
                && SPOOL_REPLAY_SNAPSHOT_NONCE.load(Ordering::Acquire) == ts.time_set_seq
            {
                SPOOL_REPLAY_CONFIRMED_SESSION.store(inbound_session_generation, Ordering::Release);
                SPOOL_REPLAY_CONFIRMED_NONCE.store(ts.time_set_seq, Ordering::Release);
            } else {
                policy_downlink::note_phone_time_sync(ts.phone_unix_ms, ts.time_set_seq);
            }
            let reply = time_sync_reply(&ts, NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed));
            if !send_mandatory_frame(reply) {
                return Err(anyhow!("TimeSync response delivery unavailable"));
            }
        }
        InboundWork::SoftApConfig(data) => {
            if !packlink_trust::is_trusted() {
                return Err(anyhow!("SoftApConfig rejected: phone not trusted"));
            }
            let frame = wardriver_packlink::unpack_frame(&data)?;
            let cfg = unpack_softap_config(frame.payload)?;
            packlink_trust::queue_softap_config(cfg.ssid, cfg.password)?;
        }
        InboundWork::TrustForget => {
            packlink_trust::clear_trust()?;
        }
    }
    Ok(())
}

fn spawn_tcp_server_thread() {
    spiram_thread::spawn_named(
        "gw-packlink-tcp",
        spiram_thread::PACKLINK_TCP_STACK_BYTES,
        tcp_accept_loop,
    )
    .expect("spawn packlink tcp thread");
}

fn tcp_accept_loop() {
    task_wdt::register_current_task_for_watchdog();
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, PACKLINK_TCP_PORT));
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            warn!("packlink softap: tcp bind :{PACKLINK_TCP_PORT} failed: {e}");
            return;
        }
    };
    if listener.set_nonblocking(true).is_err() {
        warn!("packlink softap: tcp nonblocking failed");
        return;
    }
    info!("packlink softap: tcp accept thread ready");

    loop {
        match listener.accept() {
            Ok((stream, peer)) => {
                info!("packlink softap: tcp accept from {peer}");
                if let Err(e) = handle_phone_session(stream) {
                    warn!("packlink softap: session ended: {e:#}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => warn!("packlink softap: accept: {e}"),
        }
        task_wdt::sleep_feeding_wdt(Duration::from_millis(50));
    }
}

fn handle_phone_session(mut stream: TcpStream) -> Result<()> {
    let _guard = SessionScope::enter();

    let _ = stream.set_read_timeout(Some(Duration::from_millis(SESSION_IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(SESSION_IO_TIMEOUT_MS)));

    run_tofu_handshake(&mut stream)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(SESSION_IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(SESSION_IO_TIMEOUT_MS)));

    if !clear_outbound_queue() {
        return Err(anyhow!("outbound queue reset unavailable"));
    }
    SESSION_ABORT_REQUESTED.store(false, Ordering::Release);
    let generation = SESSION_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
        .max(1);
    SESSION_GENERATION.store(generation, Ordering::Release);
    reset_spool_replay_snapshot();
    set_phone_connected(true);
    // Rotate control_boot_id before the welcome flush so the re-sent ControlWelcome
    // frames carry the new boot id; each sniffer detects the session change
    // (cmd.control_boot_id) and re-adds its ESP-NOW peer (session re-sync).
    crate::run::rotate_gateway_boot_id();
    // Re-send ControlWelcome to all rostered sniffers so they clear link_reset_pending
    // and resume observation reporting. Mirrors boot path (run.rs:173).
    if let Some(fleet) = crate::run::fleet_state() {
        crate::policy_downlink::enqueue_boot_welcomes(fleet);
    }
    // GatewayInfo before spool flush so the phone sees handshake order: TrustOk → GatewayInfo → SpoolReplay.
    if !send_frame(gateway_info_bytes()) {
        return Err(anyhow!("GatewayInfo enqueue failed"));
    }
    if let Some(hook) = CONNECT_FLUSH_HOOK.get() {
        hook();
    }

    let mut buf = vec![0u8; 4096];
    loop {
        if SESSION_ABORT_REQUESTED.load(Ordering::Acquire) {
            return Err(anyhow!("mandatory PackLink delivery unavailable"));
        }
        if let Err(e) = drain_outbound_from_queue(&mut stream) {
            warn!("packlink softap: outbound drain: {e:#}");
            break;
        }

        match read_packlink_frame(&mut stream, &mut buf) {
            Ok(frame_bytes) => {
                if let Err(e) = dispatch_inbound(frame_bytes) {
                    warn!("packlink softap: inbound dispatch: {e:#}");
                    let detail = e.to_string();
                    if detail.contains("TrustForget") || detail.contains("delivery unavailable") {
                        break;
                    }
                }
            }
            Err(e) if is_io_wait(&e) => {
                task_wdt::sleep_feeding_wdt(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
        task_wdt::reset_if_subscribed();
    }
    Ok(())
}

struct SessionScope;

impl SessionScope {
    fn enter() -> Self {
        Self
    }
}

impl Drop for SessionScope {
    fn drop(&mut self) {
        set_phone_connected(false);
        drain_or_discard_outbound_on_disconnect();
        crate::gateway_softap::deauth_all_softap_stations();
    }
}

fn drain_or_discard_outbound_on_disconnect() {
    let Some(mut queue) = try_lock_wdt(outbound_queue()) else {
        warn!("packlink softap: unable to drain outbound queue on disconnect");
        return;
    };
    while let Some(frame) = queue.pop_front() {
        match frame_msg_type(&frame) {
            Some(MsgType::ObservationBulk | MsgType::EventNotify) => {
                if let Err(e) = psram_spool::append(&frame) {
                    warn!("packlink softap: re-spool unsent frame failed: {e:#}");
                }
            }
            Some(MsgType::SnifferStatus) => {}
            Some(MsgType::SpoolReplay) => {
                // The original bytes remain in PSRAM until application confirmation.
            }
            kind => {
                warn!("packlink softap: dropping session-scoped unsent control frame type={kind:?}")
            }
        }
    }
}

fn is_io_wait(e: &anyhow::Error) -> bool {
    let s = e.to_string();
    s.contains("timed out") || s.contains("WouldBlock")
}

fn drain_outbound_from_queue(stream: &mut TcpStream) -> Result<()> {
    for _ in 0..SESSION_DRAIN_MAX_FRAMES {
        let frame = try_lock_wdt(outbound_queue()).and_then(|mut queue| queue.pop_front());
        let Some(frame) = frame else {
            break;
        };
        if let Err(e) = write_all(stream, &frame) {
            if is_mandatory_frame(&frame) {
                let _ = send_mandatory_frame(frame);
            } else {
                let _ = enqueue_outbound(frame, true);
            }
            return Err(e);
        }
        note_spool_replay_written(&frame);
    }
    Ok(())
}

fn note_spool_replay_written(frame_bytes: &[u8]) {
    let Ok(frame) = wardriver_packlink::unpack_frame(frame_bytes) else {
        return;
    };
    if frame.msg_type != MsgType::SpoolReplay {
        return;
    }
    let Ok(chunk) = wardriver_packlink::unpack_spool_replay(frame.payload) else {
        return;
    };
    let end = chunk
        .spool_offset
        .saturating_add(chunk.payload.len() as u32);
    if SPOOL_REPLAY_SNAPSHOT_SESSION.load(Ordering::Acquire) == session_generation()
        && SPOOL_REPLAY_SNAPSHOT_NONCE.load(Ordering::Acquire) == frame.seq
    {
        SPOOL_REPLAY_WRITTEN_END.fetch_max(end, Ordering::Release);
    }
}

fn run_tofu_handshake(stream: &mut TcpStream) -> Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(HANDSHAKE_IO_TIMEOUT_MS)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(HANDSHAKE_IO_TIMEOUT_MS)));

    let hello_wire = read_packlink_frame_vec(stream, HANDSHAKE_IO_TIMEOUT_MS)?;
    let hello_frame = wardriver_packlink::unpack_frame(&hello_wire)?;
    if hello_frame.msg_type != MsgType::PhoneHello {
        return Err(anyhow!("expected PhoneHello"));
    }
    let hello = unpack_phone_hello(hello_frame.payload)?;
    info!(
        "packlink softap: phone hello phone_id={} defer submit",
        hello.phone_id
    );

    let defer = NVS_DEFER
        .get()
        .ok_or_else(|| anyhow!("packlink NVS defer queue not registered"))?;
    match defer.submit_verify_phone_hello(
        &hello.phone_id,
        Duration::from_millis(PACKLINK_NVS_DEFER_WAIT_MS),
    ) {
        Ok(PacklinkHandshakeReply::TrustOk(wire)) => {
            write_all_flush(stream, &wire)?;
            info!("packlink softap: trust_ok sent");
            Ok(())
        }
        Ok(PacklinkHandshakeReply::TrustReject(wire)) => {
            write_all_flush(stream, &wire)?;
            warn!(
                "packlink softap: trust_reject sent phone_id={}",
                hello.phone_id
            );
            Err(anyhow!("TOFU rejected phone_id={}", hello.phone_id))
        }
        Err(e) => {
            warn!("packlink softap: tofu defer failed: {e}");
            Err(anyhow!("TOFU defer failed: {e}"))
        }
    }
}

fn dispatch_inbound(frame_bytes: &[u8]) -> Result<()> {
    let frame = wardriver_packlink::unpack_frame(frame_bytes)?;
    match frame.msg_type {
        MsgType::PolicySync => {
            if !enqueue_inbound(InboundWork::PolicySync(
                frame_bytes.to_vec(),
                session_generation(),
            )) {
                return Err(anyhow!("PolicySync inbound delivery unavailable"));
            }
        }
        MsgType::ChannelPlanSync => {
            if !enqueue_inbound(InboundWork::ChannelPlanSync(
                frame_bytes.to_vec(),
                session_generation(),
            )) {
                return Err(anyhow!("ChannelPlanSync inbound delivery unavailable"));
            }
        }
        MsgType::ControlCmd => {
            if !enqueue_inbound(InboundWork::ControlCmd(frame_bytes.to_vec())) {
                return Err(anyhow!("ControlCmd inbound delivery unavailable"));
            }
        }
        MsgType::CaptureCmd => {
            if !enqueue_inbound(InboundWork::CaptureCmd(frame_bytes.to_vec())) {
                return Err(anyhow!("CaptureCmd inbound delivery unavailable"));
            }
        }
        MsgType::TimeSync => {
            if !enqueue_inbound(InboundWork::TimeSync(
                frame_bytes.to_vec(),
                session_generation(),
            )) {
                return Err(anyhow!("TimeSync inbound delivery unavailable"));
            }
        }
        MsgType::SoftApConfig => {
            if !enqueue_inbound(InboundWork::SoftApConfig(frame_bytes.to_vec())) {
                return Err(anyhow!("SoftApConfig inbound delivery unavailable"));
            }
        }
        MsgType::TrustForget => {
            if enqueue_inbound(InboundWork::TrustForget) {
                return Err(anyhow!("TrustForget — closing session"));
            }
            warn!("packlink softap: TrustForget retained session because work was not queued");
        }
        _ => {}
    }
    Ok(())
}

fn read_packlink_frame_vec(stream: &mut TcpStream, timeout_ms: u64) -> Result<Vec<u8>> {
    let mut hdr = [0u8; 14];
    read_exact(stream, &mut hdr, timeout_ms)?;
    if hdr[0..4] != MAGIC {
        return Err(anyhow!("handshake bad magic"));
    }
    let payload_len = u16::from_le_bytes([hdr[8], hdr[9]]) as usize;
    let total = 14 + payload_len + 4;
    let mut buf = vec![0u8; total];
    buf[..14].copy_from_slice(&hdr);
    if total > 14 {
        read_exact(stream, &mut buf[14..], timeout_ms)?;
    }
    wardriver_packlink::unpack_frame(&buf)?;
    Ok(buf)
}

fn read_packlink_frame<'a>(stream: &mut TcpStream, scratch: &'a mut Vec<u8>) -> Result<&'a [u8]> {
    let wire = read_packlink_frame_vec(stream, SESSION_IO_TIMEOUT_MS)?;
    scratch.clear();
    scratch.extend_from_slice(&wire);
    Ok(&scratch[..])
}

fn read_exact(stream: &mut TcpStream, buf: &mut [u8], timeout_ms: u64) -> Result<()> {
    let mut off = 0;
    let deadline = stats::now_ms().saturating_add(timeout_ms);
    while off < buf.len() {
        if stats::now_ms() > deadline {
            return Err(anyhow!("read timed out"));
        }
        match stream.read(&mut buf[off..]) {
            Ok(0) => return Err(anyhow!("tcp eof")),
            Ok(n) => off += n,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                task_wdt::sleep_feeding_wdt(Duration::from_millis(5));
            }
            Err(e) => return Err(e.into()),
        }
        task_wdt::reset_if_subscribed();
    }
    Ok(())
}

fn write_all(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    write_frame_attempt(stream, data, false)
}

fn write_all_flush(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    write_frame_attempt(stream, data, true)
}

fn write_frame_attempt(stream: &mut TcpStream, data: &[u8], flush: bool) -> Result<()> {
    let started_ms = stats::now_ms();
    let result = stream
        .write_all(data)
        .and_then(|()| if flush { stream.flush() } else { Ok(()) });
    task_wdt::reset_if_subscribed();
    if stats::now_ms().saturating_sub(started_ms) >= SLOW_WRITE_MS {
        task_wdt::sleep_feeding_wdt(Duration::from_millis(5));
    }
    result.map_err(Into::into)
}
