//! wolfPACK encrypted ESP-NOW peer pool (≤ [`ESPNOW_MAX_ENCRYPT_PEERS`] concurrent).
//!
//! Pool invariants:
//! - Tracks encrypted unicast sniffers only (not broadcast `ff:ff:…`).
//! - Map size never exceeds [`ESPNOW_MAX_ENCRYPT_PEERS`] when inserting a new MAC.
//! - `last_touch_ms` advances on every `ensure_peer`; LRU eviction picks the oldest touch
//!   among peers outside the caller exclude set and not blocked for eviction.
//! - `evict_peer` keeps the IDF peer table and this map aligned (`del_peer` + remove).
//! - Post-dump: transport-end arms grace; [`tick_ingest_post_dump_phase`] on `espnow-ingest`
//!   (post-mutex) runs flush, [`try_finish_post_dump`] (`PostDumpDone` then extra Ping),
//!   and [`tick_evict_pending`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};
use esp_idf_sys::wifi_interface_t;
use log::{info, warn};

use wardriver_core::peer_join_queue::{self, AdmitDecision, PeerJoinQueue};
use wardriver_proto::{self as proto, ControlCmd, PeerJoinNackReason};
use wardriver_shared::espnow;
use wardriver_stats as stats;
use wardriver_stats::fleet::ESPNOW_MAX_ENCRYPT_PEERS;

use crate::gateway_downlink;
use crate::ingest;

/// Control/gateway hook: deferred post-dump downlink (SetRole + channel schedule).
pub type PostDumpFlushFn = fn([u8; 6], &AtomicU32, u32, wifi_interface_t, Option<u32>);

static POST_DUMP_FLUSH: OnceLock<PostDumpFlushFn> = OnceLock::new();

/// Register role/schedule flush after transport-end reasm (control wires channel plan).
pub fn register_post_dump_flush(f: PostDumpFlushFn) {
    let _ = POST_DUMP_FLUSH.set(f);
}

/// Sniffer listen window after dump (~600 ms) plus margin for encrypted downlink.
pub const POST_DUMP_GRACE_MS: u64 = 650;
/// Evict soon after `PostDumpDone` TX (replaces full grace when done sequence completes).
pub const POST_DUMP_DONE_GRACE_MS: u64 = 100;
/// Scan-only [`PeerJoinReason::PeerRefresh`] visits: release encrypted peer soon after `SnifferAck`.
pub const PEER_REFRESH_GRACE_MS: u64 = POST_DUMP_DONE_GRACE_MS;

/// Colon-hex uppercase MAC for serial logs (grep-friendly, ASCII-only).
fn mac_hex(mac: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

#[derive(Debug, Clone, Copy)]
struct PeerSlot {
    channel: u8,
    ifidx: wifi_interface_t,
    last_touch_ms: u64,
    /// `Some(deadline)` after transport-end reasm; grace eviction via [`tick_evict_pending`].
    pending_evict_at: Option<u64>,
    /// Set after successful `PeerJoin` ack until first bulk frag or [`VISIT_ARM_TIMEOUT_MS`].
    visit_armed_ms: Option<u64>,
}

/// Per-MAC post-dump finish state (wolfPACK).
#[derive(Debug, Clone, Copy)]
struct PostDumpSession {
    sniffer_id: u32,
    /// Wall time of the first `PeerJoin` for this visit (before FIFO admit).
    join_attempt_ms: u64,
    /// `PeerJoinAck` time; also the visit anchor for legacy `visit_ms`.
    visit_start_ms: u64,
    /// First bulk frag RX after admit.
    bulk_start_ms: u64,
    /// Last frag with `FRAG_FLAG_TRANSPORT_END` (or zero-frags ack path).
    transport_end_ms: u64,
    flush_done: bool,
    done_sent: bool,
    /// True when this visit sent a TimeSyncPing (RTT due).
    time_sync_attempted: bool,
    /// Pong handled + TimeSet sent, skipped, or rejected.
    time_sync_done: bool,
    /// [`proto::PeerJoinReason`] wire value for this visit (`0` = unset).
    join_reason: u8,
}

impl Default for PostDumpSession {
    fn default() -> Self {
        Self {
            sniffer_id: 0,
            join_attempt_ms: 0,
            visit_start_ms: 0,
            bulk_start_ms: 0,
            transport_end_ms: 0,
            flush_done: false,
            done_sent: false,
            time_sync_attempted: false,
            time_sync_done: false,
            join_reason: 0,
        }
    }
}

static PEER_POOL: OnceLock<Mutex<HashMap<[u8; 6], PeerSlot>>> = OnceLock::new();
static POST_DUMP_SESSIONS: OnceLock<Mutex<HashMap<[u8; 6], PostDumpSession>>> = OnceLock::new();
static POST_DUMP_SESSION_SEQ: AtomicU32 = AtomicU32::new(1);
static PEER_JOIN_QUEUE: OnceLock<Mutex<PeerJoinQueue>> = OnceLock::new();

/// Dependencies for [`try_finish_post_dump`] (unicast done + broadcast Ping).
pub struct PostDumpFinishDeps<'a> {
    pub cmd_seq: &'a AtomicU32,
    pub control_boot_id: u32,
    pub peer_if: wifi_interface_t,
    pub sniffers: &'a Arc<Mutex<HashSet<[u8; 6]>>>,
    pub scanning_allowed: bool,
    pub time_set_seq: Option<u32>,
}

/// Handles for [`tick_ingest_post_dump_phase`] on the `espnow-ingest` worker.
pub struct IngestPostDumpDeps<'a> {
    pub cmd_seq: &'a AtomicU32,
    pub control_boot_id: u32,
    pub peer_if: wifi_interface_t,
    pub sniffers: &'a Arc<Mutex<HashSet<[u8; 6]>>>,
    pub scanning_allowed: &'a AtomicBool,
}

/// Register shared ingest maps so cap eviction skips MACs with in-flight reasm.
pub fn register_ingest(ingest: Arc<Mutex<dyn ingest::IngestPeerPoolState>>) {
    ingest::register_ingest(ingest);
}

fn cap_has_incomplete_reasm(mac: &[u8; 6]) -> bool {
    ingest::with_ingest(|g| g.has_incomplete_reasm_for_mac(mac))
        // Conservative: skip LRU eviction when ingest mutex is contended.
        .unwrap_or(true)
}

fn pool() -> &'static Mutex<HashMap<[u8; 6], PeerSlot>> {
    PEER_POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn post_dump_sessions() -> &'static Mutex<HashMap<[u8; 6], PostDumpSession>> {
    POST_DUMP_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn join_queue() -> &'static Mutex<PeerJoinQueue> {
    PEER_JOIN_QUEUE.get_or_init(|| Mutex::new(PeerJoinQueue::new()))
}
fn new_peer_slot(channel: u8, ifidx: wifi_interface_t, last_touch_ms: u64) -> PeerSlot {
    PeerSlot {
        channel,
        ifidx,
        last_touch_ms,
        pending_evict_at: None,
        visit_armed_ms: None,
    }
}

fn with_session<F, R>(mac: [u8; 6], f: F) -> R
where
    F: FnOnce(&mut PostDumpSession) -> R,
{
    let mut guard = post_dump_sessions().lock().ok();
    let Some(g) = guard.as_mut() else {
        return f(&mut PostDumpSession::default());
    };
    let ent = g.entry(mac).or_default();
    f(ent)
}

/// True when `mac` still has in-flight wifi/ble reasm (caller supplies ingress map lookup).
pub fn peer_evict_blocked(mac: &[u8; 6], has_incomplete: impl Fn(&[u8; 6]) -> bool) -> bool {
    has_incomplete(mac)
}
fn visit_arm_blocks(slot: &PeerSlot, now_ms: u64) -> bool {
    slot.visit_armed_ms
        .is_some_and(|t| now_ms < t.saturating_add(peer_join_queue::VISIT_ARM_TIMEOUT_MS))
}
fn clear_visit_arm(mac: &[u8; 6]) {
    if let Ok(mut g) = pool().lock() {
        if let Some(slot) = g.get_mut(mac) {
            slot.visit_armed_ms = None;
        }
    }
}
fn arm_visit(mac: [u8; 6], now_ms: u64) {
    if let Ok(mut g) = pool().lock() {
        if let Some(slot) = g.get_mut(&mac) {
            slot.visit_armed_ms = Some(now_ms);
        }
    }
}

/// wolfPACK visit-arm blocks LRU eviction while a dump visit is armed.
fn visit_guard_allows_evict(slot: &PeerSlot, now_ms: u64) -> bool {
    !visit_arm_blocks(slot, now_ms)
}

fn lru_evictable(
    mac: &[u8; 6],
    slot: &PeerSlot,
    exclude: &HashSet<[u8; 6]>,
    has_incomplete: &impl Fn(&[u8; 6]) -> bool,
    now_ms: u64,
) -> bool {
    !exclude.contains(mac)
        && !peer_evict_blocked(mac, has_incomplete)
        && slot.pending_evict_at.is_none()
        && visit_guard_allows_evict(slot, now_ms)
}

/// Register or refresh an encrypted peer; evict LRU idle peer when at cap.
pub fn ensure_peer(mac: [u8; 6], channel: u8, ifidx: wifi_interface_t) -> Result<()> {
    let now = stats::now_ms();
    let sid = stats::sniffer_id_for_mac(&mac);
    let mut guard = pool()
        .lock()
        .map_err(|_| anyhow!("peer_pool mutex poison"))?;

    if let Some(slot) = guard.get_mut(&mac) {
        slot.channel = channel;
        slot.ifidx = ifidx;
        slot.last_touch_ms = now;
        drop(guard);
        // Re-adoption/uplink must refresh liveness or tick_stale_liveness_evict evicts
        // in the same tick (dead-on-arrival race).
        if let Some(sid) = sid {
            stats::sniffer_seen_liveness(sid, &mac);
        }
        return espnow::add_peer(mac, channel, ifidx);
    }

    if guard.len() >= ESPNOW_MAX_ENCRYPT_PEERS {
        drop(guard);
        stats::PEER_POOL_CAP.fetch_add(1, Ordering::Relaxed);
        if !evict_lru_idle(&HashSet::new(), cap_has_incomplete_reasm) {
            warn!(
                "peer_pool: at cap ({ESPNOW_MAX_ENCRYPT_PEERS}); no evictable idle peer for {:02x}:{:02x}:…",
                mac[0], mac[1]
            );
            return Err(anyhow!("peer_pool at cap; no idle peer to evict"));
        }
        guard = pool()
            .lock()
            .map_err(|_| anyhow!("peer_pool mutex poison"))?;
    }

    guard.insert(mac, new_peer_slot(channel, ifidx, now));
    drop(guard);
    stats::PEER_POOL_JOIN.fetch_add(1, Ordering::Relaxed);
    log_peer_uplink_transition(&mac, now);
    // Re-adoption/uplink must refresh liveness or tick_stale_liveness_evict evicts
    // in the same tick (dead-on-arrival race). Refresh AFTER the resume log so the
    // log reports the true pre-arrival age.
    if let Some(sid) = sid {
        stats::sniffer_seen_liveness(sid, &mac);
    }
    espnow::add_peer(mac, channel, ifidx)
}

/// Edge-triggered log on a fresh/resume peer add: first uplink vs uplink resumed.
/// Fires only when the MAC is absent from the pool before insert (the insert branch).
fn log_peer_uplink_transition(mac: &[u8; 6], now_ms: u64) {
    let sid = stats::sniffer_id_for_mac(mac).unwrap_or(0);
    let last_seen = stats::sniffer_last_seen_ms(sid).unwrap_or(0);
    if last_seen == 0 {
        info!("gateway: sniffer {sid} ({}) first uplink", mac_hex(mac));
    } else {
        let age_ms = now_ms.saturating_sub(last_seen);
        info!(
            "gateway: sniffer {sid} ({}) uplink resumed (last seen {age_ms}ms ago)",
            mac_hex(mac)
        );
    }
}

/// Remove `mac` from the IDF peer table and pool map (immediate evict).
pub fn evict_peer(mac: [u8; 6]) -> Result<()> {
    let removed = match pool().lock() {
        Ok(mut g) => g.remove(&mac).is_some(),
        Err(_) => false,
    };
    if let Ok(mut g) = post_dump_sessions().lock() {
        g.remove(&mac);
    }
    espnow::del_peer(mac)?;
    if removed {
        stats::PEER_POOL_EVICT.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// Evict the oldest idle peer not in `exclude`. Returns `true` when a peer was removed.
pub fn evict_lru_idle(
    exclude: &HashSet<[u8; 6]>,
    has_incomplete: impl Fn(&[u8; 6]) -> bool,
) -> bool {
    let now_ms = stats::now_ms();
    let victim = pool().lock().ok().and_then(|g| {
        g.iter()
            .filter(|(mac, slot)| lru_evictable(mac, slot, exclude, &has_incomplete, now_ms))
            .min_by_key(|(_, slot)| slot.last_touch_ms)
            .map(|(mac, _)| *mac)
    });
    let Some(mac) = victim else {
        return false;
    };
    match evict_peer(mac) {
        Ok(()) => true,
        Err(e) => {
            warn!(
                "peer_pool: evict_lru_idle {:02x}:{:02x}:… failed: {e:#}",
                mac[0], mac[1]
            );
            false
        }
    }
}

/// Record the first `PeerJoin` for a dump visit (queue wait anchor).
///
/// Called on every admitted roster `PeerJoin` before FIFO admit. Repeated joins
/// while queued keep the original `join_attempt_ms`.
pub fn note_peer_join_attempt(mac: [u8; 6], sniffer_id: u32, now_ms: u64, join_reason: u8) {
    with_session(mac, |s| {
        if s.done_sent {
            // Prior visit finished; start a fresh attempt.
            *s = PostDumpSession {
                sniffer_id,
                join_attempt_ms: now_ms,
                join_reason,
                ..PostDumpSession::default()
            };
            return;
        }
        if s.join_attempt_ms == 0 {
            s.join_attempt_ms = now_ms;
            s.sniffer_id = sniffer_id;
        }
        // Repeat `PeerJoin` while queued or refreshing: keep reason current.
        s.join_reason = join_reason;
    });
}

/// Begin dump visit timing on successful `PeerJoinAck` (`visit_start_ms`).
pub fn begin_post_dump_visit(mac: [u8; 6], sniffer_id: u32, now_ms: u64, join_reason: u8) {
    with_session(mac, |s| {
        let join_attempt_ms = s.join_attempt_ms;
        *s = PostDumpSession {
            sniffer_id,
            join_attempt_ms,
            visit_start_ms: now_ms,
            join_reason,
            ..PostDumpSession::default()
        };
    });
}

/// First bulk frag RX after join (bulk_start_ms).
pub fn note_bulk_frag(mac: [u8; 6], now_ms: u64) {
    clear_visit_arm(&mac);
    with_session(mac, |s| {
        if s.bulk_start_ms == 0 {
            s.bulk_start_ms = now_ms;
        }
    });
}

fn set_transport_end_session(mac: [u8; 6], now_ms: u64) {
    with_session(mac, |s| {
        if s.transport_end_ms == 0 {
            s.transport_end_ms = now_ms;
        }
    });
}

/// After `flush_post_dump_downlink` on transport-end path.
pub fn mark_flush_done(mac: [u8; 6]) {
    with_session(mac, |s| {
        s.flush_done = true;
    });
}

/// Arm post-dump grace after transport-end reasm; returns eviction deadline ms.
pub fn mark_transport_end(mac: [u8; 6], now_ms: u64) -> Option<u64> {
    set_transport_end_session(mac, now_ms);
    arm_post_dump_grace(mac, now_ms)
}

/// True when `mac` has an admitted scan-only refresh visit awaiting `SnifferAck`.
pub fn is_active_peer_refresh(mac: &[u8; 6]) -> bool {
    let mut guard = post_dump_sessions().lock().ok();
    let Some(g) = guard.as_mut() else {
        return false;
    };
    let Some(s) = g.get(mac) else {
        return false;
    };
    s.join_reason == proto::PeerJoinReason::PeerRefresh as u8
        && s.visit_start_ms != 0
        && s.bulk_start_ms == 0
        && s.transport_end_ms == 0
}

/// Close a scan-only refresh visit after encrypted `SnifferAck` (no `PostDumpDone`).
pub fn complete_peer_refresh_visit(mac: [u8; 6], now_ms: u64) {
    if !is_active_peer_refresh(&mac) {
        return;
    }
    clear_visit_arm(&mac);
    with_session(mac, |s| {
        if s.transport_end_ms == 0 {
            s.transport_end_ms = now_ms;
        }
        // Terminal on SnifferAck: no PostDumpDone / dump telemetry.
        s.flush_done = true;
        s.done_sent = true;
    });
    let deadline = now_ms.saturating_add(PEER_REFRESH_GRACE_MS);
    if let Ok(mut g) = pool().lock() {
        if let Some(slot) = g.get_mut(&mac) {
            slot.pending_evict_at = Some(deadline);
            slot.visit_armed_ms = None;
        }
    }
    stats::PEER_REFRESH_COMPLETE.fetch_add(1, Ordering::Relaxed);
}

fn arm_post_dump_grace(mac: [u8; 6], now_ms: u64) -> Option<u64> {
    let pending_evict_at = now_ms.saturating_add(POST_DUMP_GRACE_MS);
    let mut guard = pool().lock().ok()?;
    let slot = guard.get_mut(&mac)?;
    slot.visit_armed_ms = None;
    slot.pending_evict_at = Some(pending_evict_at);
    stats::PEER_POOL_TRANSPORT_END.fetch_add(1, Ordering::Relaxed);
    Some(pending_evict_at)
}

fn shorten_grace_after_done(mac: [u8; 6], now_ms: u64) {
    let deadline = now_ms.saturating_add(POST_DUMP_DONE_GRACE_MS);
    if let Ok(mut g) = pool().lock() {
        if let Some(slot) = g.get_mut(&mac) {
            slot.pending_evict_at = Some(deadline);
        }
    }
}

/// Pure policy: ready to send `PostDumpDone` after flush (and RTT when attempted).
pub fn finish_ready(
    transport_end_ms: u64,
    flush_done: bool,
    done_sent: bool,
    time_sync_attempted: bool,
    time_sync_done: bool,
    now_ms: u64,
) -> bool {
    if transport_end_ms == 0 || !flush_done || done_sent {
        return false;
    }
    crate::time_sync_rtt::post_dump_time_sync_ready(
        time_sync_attempted,
        time_sync_done,
        transport_end_ms,
        now_ms,
    )
}
pub fn mark_post_dump_time_sync_attempted(mac: [u8; 6], _seq: u32) {
    with_session(mac, |s| {
        s.time_sync_attempted = true;
        s.time_sync_done = false;
    });
}
pub fn mark_post_dump_time_sync_skipped(mac: [u8; 6]) {
    with_session(mac, |s| {
        s.time_sync_attempted = false;
        s.time_sync_done = true;
    });
}
pub fn mark_post_dump_time_sync_done(mac: [u8; 6]) {
    with_session(mac, |s| {
        s.time_sync_done = true;
    });
}

fn session_pending_post_dump_finish(s: &PostDumpSession) -> bool {
    s.transport_end_ms != 0
        && s.flush_done
        && !s.done_sent
        && s.join_reason != proto::PeerJoinReason::PeerRefresh as u8
}

/// Run finish sequence for `mac` when [`finish_ready`]. Returns `true` if done was sent this call.
pub fn try_finish_post_dump(mac: [u8; 6], now_ms: u64, deps: &PostDumpFinishDeps<'_>) -> bool {
    let snapshot = with_session(mac, |s| *s);
    if snapshot.join_reason == proto::PeerJoinReason::PeerRefresh as u8 {
        return false;
    }
    if !finish_ready(
        snapshot.transport_end_ms,
        snapshot.flush_done,
        snapshot.done_sent,
        snapshot.time_sync_attempted,
        snapshot.time_sync_done,
        now_ms,
    ) {
        return false;
    }
    if snapshot.sniffer_id == 0 {
        return false;
    }

    let session_seq = POST_DUMP_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let control_tx_ms = now_ms.min(u64::from(u32::MAX)) as u32;
    if let Err(e) = gateway_downlink::send_post_dump_done(
        mac,
        snapshot.sniffer_id,
        session_seq,
        control_tx_ms,
        deps.peer_if,
    ) {
        stats::POST_DUMP_DONE_TX_FAIL.fetch_add(1, Ordering::Relaxed);
        warn!(
            "peer_pool: post_dump_done sniffer_id={}: {e:#}",
            snapshot.sniffer_id
        );
        return false;
    }
    stats::POST_DUMP_DONE_TX.fetch_add(1, Ordering::Relaxed);

    let durations = wardriver_core::wolfpack_telemetry::wolfpack_dump_durations(
        snapshot.join_attempt_ms,
        snapshot.visit_start_ms,
        snapshot.bulk_start_ms,
        snapshot.transport_end_ms,
        now_ms,
    );
    // Legacy bulk_ms: first frag → PostDumpDone (unchanged for JSON compat).
    let bulk_ms = if snapshot.bulk_start_ms != 0 {
        now_ms.saturating_sub(snapshot.bulk_start_ms)
    } else {
        0
    };
    stats::record_dump_complete(
        snapshot.sniffer_id,
        durations.visit_ms,
        bulk_ms,
        durations.post_ms,
        now_ms,
        durations.queue_ms,
        durations.bulk_xfer_ms,
    );

    with_session(mac, |s| {
        s.done_sent = true;
    });
    shorten_grace_after_done(mac, now_ms);

    let desired_scanning = u8::from(deps.scanning_allowed);
    if let Err(e) = gateway_downlink::broadcast_cmd(
        deps.sniffers,
        deps.cmd_seq,
        ControlCmd::Ping,
        None,
        deps.peer_if,
        deps.control_boot_id,
        true,
        &gateway_downlink::FleetCommandBuilder::new(0),
        deps.time_set_seq,
        desired_scanning,
    ) {
        warn!("peer_pool: post-dump extra ping: {e:#}");
    }
    true
}
static POST_DUMP_FLUSH_QUEUE: OnceLock<Mutex<Vec<[u8; 6]>>> = OnceLock::new();
fn flush_queue() -> &'static Mutex<Vec<[u8; 6]>> {
    POST_DUMP_FLUSH_QUEUE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Queue `mac` for ingest post-mutex `flush_post_dump_downlink` (dedupe by MAC).
pub fn enqueue_post_dump_flush(mac: [u8; 6]) {
    if let Ok(mut q) = flush_queue().lock() {
        if !q.contains(&mac) {
            q.push(mac);
        }
    }
}

/// True when transport-end flush downlink is queued but not yet sent.
pub fn has_pending_post_dump_flush() -> bool {
    flush_queue().lock().ok().is_some_and(|q| !q.is_empty())
}

/// True when post-dump flush and/or finish work remains for the ingest worker.
pub fn has_pending_post_dump_work() -> bool {
    has_pending_post_dump_flush() || has_pending_post_dump_finish()
}

/// Post-dump flush, finish, and grace eviction on `espnow-ingest` (after ingest mutex released).
pub fn tick_ingest_post_dump_phase(deps: &IngestPostDumpDeps<'_>) {
    let time_set_seq = Some(stats::time_set_seq_for_wire());
    tick_post_dump_flush_queue(
        deps.cmd_seq,
        deps.control_boot_id,
        deps.peer_if,
        time_set_seq,
    );
    let now_ms = stats::now_ms();
    let scan_allowed = deps.scanning_allowed.load(Ordering::Acquire);
    let finish_deps = PostDumpFinishDeps {
        cmd_seq: deps.cmd_seq,
        control_boot_id: deps.control_boot_id,
        peer_if: deps.peer_if,
        sniffers: deps.sniffers,
        scanning_allowed: scan_allowed,
        time_set_seq,
    };
    for mac in pending_finish_macs() {
        let _ = try_finish_post_dump(mac, now_ms, &finish_deps);
    }
    ingest::with_ingest(|ingest_guard| {
        tick_evict_pending(now_ms, |mac| ingest_guard.has_incomplete_reasm_for_mac(mac));
    });
}

/// Drain queued MACs for [`super::fleet_cmd::flush_post_dump_downlink`].
pub fn drain_post_dump_flush_queue() -> Vec<[u8; 6]> {
    flush_queue()
        .lock()
        .ok()
        .map(|mut q| std::mem::take(&mut *q))
        .unwrap_or_default()
}

/// Send deferred post-dump downlink unicasts; marks flush done per MAC.
pub fn tick_post_dump_flush_queue(
    cmd_seq: &AtomicU32,
    control_boot_id: u32,
    peer_if: wifi_interface_t,
    time_set_seq: Option<u32>,
) {
    let now_ms = stats::now_ms();
    for mac in drain_post_dump_flush_queue() {
        let sniffer_id = stats::sniffer_id_for_mac(&mac).unwrap_or(0);
        if let Some(flush) = POST_DUMP_FLUSH.get() {
            flush(mac, cmd_seq, control_boot_id, peer_if, time_set_seq);
        }
        mark_flush_done(mac);
        crate::time_sync_rtt::maybe_start_post_dump_time_sync(mac, sniffer_id, now_ms, peer_if);
    }
}

/// True when any post-dump session awaits ingest finish.
pub fn has_pending_post_dump_finish() -> bool {
    post_dump_sessions()
        .lock()
        .ok()
        .map(|g| g.values().any(session_pending_post_dump_finish))
        .unwrap_or(false)
}

/// MACs with an in-progress post-dump session (for main-loop finish polling).
pub fn pending_finish_macs() -> Vec<[u8; 6]> {
    post_dump_sessions()
        .lock()
        .ok()
        .map(|g| {
            g.iter()
                .filter(|(_, s)| session_pending_post_dump_finish(s))
                .map(|(mac, _)| *mac)
                .collect()
        })
        .unwrap_or_default()
}

/// Evict peers whose post-dump grace expired and reasm is complete.
pub fn tick_evict_pending(now_ms: u64, has_incomplete: impl Fn(&[u8; 6]) -> bool) -> u32 {
    let due: Vec<[u8; 6]> = pool()
        .lock()
        .ok()
        .map(|g| {
            g.iter()
                .filter_map(|(mac, slot)| {
                    let deadline = slot.pending_evict_at?;
                    if now_ms < deadline || peer_evict_blocked(mac, &has_incomplete) {
                        return None;
                    }
                    Some(*mac)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut evicted = 0u32;
    for mac in due {
        if evict_peer(mac).is_ok() {
            stats::PEER_POOL_GRACE.fetch_add(1, Ordering::Relaxed);
            evicted += 1;
        }
    }
    if evicted > 0 {
        maybe_request_queue_hint_ping_after_queue_change();
    }
    evicted
}

/// Active encrypted peers tracked by the pool (dashboard / debug).
pub fn active_count() -> usize {
    pool().lock().map(|g| g.len()).unwrap_or(0)
}
static QUEUE_HINT_PING_REQUESTED: AtomicBool = AtomicBool::new(false);
static LAST_QUEUE_HINT_PING_MS: AtomicU32 = AtomicU32::new(0);
/// Minimum spacing for opportunistic queue-hint Ping (reuses periodic Ping wire).
const QUEUE_HINT_PING_MIN_MS: u64 = 150;

/// Request an extra broadcast `Ping` so join waiters see fresh `peer_slots_free` / `peer_join_waiters`.
pub fn request_queue_hint_ping() {
    QUEUE_HINT_PING_REQUESTED.store(true, Ordering::Relaxed);
}

/// Returns true when an opportunistic queue-hint Ping should be sent (rate-limited).
pub fn take_queue_hint_ping_due(now_ms: u64) -> bool {
    if !QUEUE_HINT_PING_REQUESTED.swap(false, Ordering::Relaxed) {
        return false;
    }
    let last = u64::from(LAST_QUEUE_HINT_PING_MS.load(Ordering::Relaxed));
    if now_ms.saturating_sub(last) < QUEUE_HINT_PING_MIN_MS {
        QUEUE_HINT_PING_REQUESTED.store(true, Ordering::Relaxed);
        return false;
    }
    LAST_QUEUE_HINT_PING_MS.store(now_ms.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
    true
}
fn maybe_request_queue_hint_ping_after_queue_change() {
    if join_queue_waiter_count() > 0 {
        request_queue_hint_ping();
    }
}

/// Params for a broadcast [`proto::PeerJoinNack`] when admission fails.
pub struct PeerJoinNackOut {
    pub reason: PeerJoinNackReason,
    pub position: u8,
}

/// FIFO gate + `ensure_peer` for roster-adopted `PeerJoin`.
pub fn try_admit_peer_join(
    sniffer_id: u32,
    mac: [u8; 6],
    channel: u8,
    ifidx: wifi_interface_t,
) -> Result<(), PeerJoinNackOut> {
    let now_ms = stats::now_ms();
    let mut q = join_queue().lock().map_err(|_| PeerJoinNackOut {
        reason: PeerJoinNackReason::CapFull,
        position: 0,
    })?;
    let decision = q.admit_or_enqueue(sniffer_id, mac, now_ms);

    match decision {
        AdmitDecision::Wait { position, .. } => {
            return Err(PeerJoinNackOut {
                reason: PeerJoinNackReason::NotYourTurn,
                position,
            });
        }
        AdmitDecision::Admit => {}
    }

    if let Err(e) = ensure_peer(mac, channel, ifidx) {
        warn!(
            "peer_pool: peer_join ensure_peer sniffer_id={}: {e:#}",
            sniffer_id
        );
        let position = join_queue()
            .lock()
            .map(|mut q| q.enqueue_cap_full(sniffer_id, mac, now_ms))
            .unwrap_or(1);
        return Err(PeerJoinNackOut {
            reason: PeerJoinNackReason::CapFull,
            position,
        });
    }

    Ok(())
}

/// Undo a successful admit when `PeerJoinAck` TX fails: evict peer, re-enqueue, return nack payload.
pub fn abort_peer_join_after_failed_ack(
    sniffer_id: u32,
    mac: [u8; 6],
    now_ms: u64,
) -> PeerJoinNackOut {
    let _ = evict_peer(mac);
    let position = join_queue()
        .lock()
        .map(|mut q| q.enqueue_cap_full(sniffer_id, mac, now_ms))
        .unwrap_or(1);
    maybe_request_queue_hint_ping_after_queue_change();
    PeerJoinNackOut {
        reason: PeerJoinNackReason::CapFull,
        position,
    }
}

/// After successful `PeerJoinAck` TX: arm visit guard and drop from join queue.
pub fn confirm_peer_join_admitted(sniffer_id: u32, mac: [u8; 6], now_ms: u64) {
    arm_visit(mac, now_ms);
    if let Ok(mut q) = join_queue().lock() {
        q.on_admitted(sniffer_id, &mac);
    }
    // Remaining waiters need updated hints (queue depth dropped).
    maybe_request_queue_hint_ping_after_queue_change();
}

/// Broadcast unencrypted `PeerJoinNack` (targeted by `sniffer_id` in payload).
pub fn send_peer_join_nack(sniffer_id: u32, nack: &PeerJoinNackOut, peer_if: wifi_interface_t) {
    let wire = proto::pack_peer_join_nack(proto::PeerJoinNackV1 {
        sniffer_id,
        reason: nack.reason,
        retry_after_ms: peer_join_queue::NACK_RETRY_AFTER_MS,
        queue_position: nack.position,
    });
    if let Err(e) = espnow::add_broadcast_peer(crate::CONTROL_AP_CHANNEL, peer_if) {
        warn!("peer_pool: peer_join_nack add_broadcast_peer: {e:#}");
    }
    match espnow::send(espnow::BROADCAST_MAC, &wire) {
        Ok(()) => {
            stats::PEER_JOIN_NACK_TX.fetch_add(1, Ordering::Relaxed);
        }
        Err(e) => {
            warn!(
                "peer_pool: peer_join_nack send sniffer_id={}: {e:#}",
                sniffer_id
            );
        }
    }
}

/// Estimated admit slots for broadcast `Ping` hint (`0..=16`).
pub fn peer_slots_free_estimate() -> u8 {
    let now_ms = stats::now_ms();
    let Ok(guard) = pool().lock() else {
        return 0;
    };
    let active = guard.len();
    if active < ESPNOW_MAX_ENCRYPT_PEERS {
        return (ESPNOW_MAX_ENCRYPT_PEERS - active).min(255) as u8;
    }
    let evictable = guard
        .iter()
        .filter(|(mac, slot)| {
            lru_evictable(
                mac,
                slot,
                &HashSet::new(),
                &cap_has_incomplete_reasm,
                now_ms,
            )
        })
        .count();
    evictable.min(255) as u8
}

/// Live FIFO join-queue depth for broadcast `Ping` hint.
pub fn join_queue_waiter_count() -> u8 {
    join_queue()
        .lock()
        .map(|q| q.live_waiter_count(stats::now_ms()))
        .unwrap_or(0)
}

/// Clear expired visit-arm guards so LRU can reclaim slots from crashed sniffers.
pub fn tick_visit_arm_expiry(now_ms: u64) -> u32 {
    let mut expired = 0u32;
    if let Ok(mut g) = pool().lock() {
        for slot in g.values_mut() {
            let Some(armed_at) = slot.visit_armed_ms else {
                continue;
            };
            if now_ms >= armed_at.saturating_add(peer_join_queue::VISIT_ARM_TIMEOUT_MS) {
                slot.visit_armed_ms = None;
                expired += 1;
                stats::PEER_VISIT_ARM_TIMEOUT.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    expired
}

/// Prune expired FIFO heads (main-loop maintenance).
pub fn tick_join_queue_prune(now_ms: u64) -> u32 {
    join_queue()
        .lock()
        .map(|mut q| {
            let n = q.prune_expired(now_ms);
            if n > 0 {
                stats::PEER_JOIN_QUEUE_EXPIRE.fetch_add(n, Ordering::Relaxed);
            }
            n
        })
        .unwrap_or(0)
}

fn drop_ingest_state_for_sniffer(sniffer_id: u32) {
    let _ = ingest::try_with_ingest_mut(|g| g.drop_state_for_sniffer(sniffer_id));
}

/// Evict encrypted peers whose roster `last_seen_ms` exceeds the uplink liveness budget.
pub fn tick_stale_liveness_evict(now_ms: u64) -> u32 {
    use wardriver_core::liveness::{stale_uplink_peer_should_evict, SNIFFER_UPLINK_LIVENESS_MS};

    let macs: Vec<([u8; 6], u64)> = pool()
        .lock()
        .map(|g| {
            g.iter()
                .map(|(mac, slot)| (*mac, slot.last_touch_ms))
                .collect()
        })
        .unwrap_or_default();

    let mut evicted = 0u32;
    for (mac, last_touch_ms) in macs {
        // A peer (re)inserted within the liveness budget must not be evicted in the
        // same tick it re-adopts (dead-on-arrival race).
        if now_ms.saturating_sub(last_touch_ms) <= SNIFFER_UPLINK_LIVENESS_MS {
            continue;
        }
        let last_seen = match stats::sniffer_id_for_mac(&mac) {
            Some(sid) => stats::sniffer_last_seen_ms(sid).unwrap_or(0),
            None => 0,
        };
        if !stale_uplink_peer_should_evict(last_seen, now_ms) {
            continue;
        }
        if let Some(sid) = stats::sniffer_id_for_mac(&mac) {
            drop_ingest_state_for_sniffer(sid);
            if let Ok(mut q) = join_queue().lock() {
                let _ = q.remove_sniffer(sid);
            }
        }
        match evict_peer(mac) {
            Ok(()) => {
                let sid = stats::sniffer_id_for_mac(&mac).unwrap_or(0);
                let age_ms = now_ms.saturating_sub(last_seen);
                warn!(
                    "gateway: sniffer {sid} ({}) stale after {age_ms}ms no uplink; evicting peer",
                    mac_hex(&mac)
                );
                stats::PEER_POOL_STALE_LIVENESS_EVICT.fetch_add(1, Ordering::Relaxed);
                evicted += 1;
            }
            Err(e) => warn!("peer_pool: stale liveness evict {:?} failed: {e:#}", mac),
        }
    }
    evicted
}
