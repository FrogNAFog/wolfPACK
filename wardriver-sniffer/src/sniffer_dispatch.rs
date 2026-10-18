//! ESP-NOW bulk transport (sniffer firmware).

use anyhow::Result;
use log::warn;

/// Post-alert / post-dump control command drain window (ms).
pub(crate) const CONTROL_CMD_WINDOW_MS: u64 = 600;

/// Finish-cycle hook for wardrive bulk telemetry (reserved; loss is counted in
/// [`crate::sniffer_outbound_bulk::OutboundBulkStore`] on GPS staleness or budget eviction).
pub fn record_bulk_cycle(_had_data: bool, _wardrive_suppressed: bool, _delivered: bool) {}

// --- wolfPACK: join-before-dump + transport-end on last frag ---

mod wolfpack {
    use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};
    use std::time::Duration;

    use esp_idf_sys as sys;
    use log::{info, warn};
    use wardriver_proto::{
        pack_peer_join, PeerJoinNackReason, PeerJoinReason, PeerJoinV1,
        SNIFFER_JOIN_PRESSURE_DEADLINE_MS,
    };
    use wardriver_shared::espnow;
    use wardriver_stats as stats;

    /// Dedupe rapid identical `join_queue` lines during bulk fail-safe retries.
    static LAST_JOIN_QUEUE_LOG_MS: AtomicU32 = AtomicU32::new(0);
    static LAST_JOIN_QUEUE_LOG_KEY: AtomicU32 = AtomicU32::new(u32::MAX);

    fn join_queue_log_key(position: u8, reason: PeerJoinNackReason) -> u32 {
        ((u32::from(position)) << 8) | (reason as u32)
    }

    /// Sniffer tuned to control channel to attempt a wardrive dump.
    pub fn trace_return_control_for_dump(
        sniffer_id: u32,
        media: &'static str,
        control_ch: u8,
        frags: u32,
        bulk_pressure: bool,
        age_pressure: bool,
    ) {
        info!(
            "wolfpack/dump: return_control sniffer_id={sniffer_id} media={media} ch={control_ch} frags={frags} bulk_pressure={bulk_pressure} age_pressure={age_pressure}"
        );
    }

    /// Control nack: sniffer is in (or behind) the FIFO join queue.
    pub fn trace_join_queue(
        sniffer_id: u32,
        media: &'static str,
        queue_position: u8,
        reason: PeerJoinNackReason,
    ) {
        let key = join_queue_log_key(queue_position, reason);
        let now_ms = stats::now_ms().min(u64::from(u32::MAX)) as u32;
        let last_ms = LAST_JOIN_QUEUE_LOG_MS.load(Ordering::Relaxed);
        let last_key = LAST_JOIN_QUEUE_LOG_KEY.load(Ordering::Relaxed);
        if key == last_key && now_ms.saturating_sub(last_ms) < 500 {
            return;
        }
        LAST_JOIN_QUEUE_LOG_KEY.store(key, Ordering::Relaxed);
        LAST_JOIN_QUEUE_LOG_MS.store(now_ms, Ordering::Relaxed);
        let reason_s = match reason {
            PeerJoinNackReason::CapFull => "cap_full",
            PeerJoinNackReason::NotYourTurn => "not_your_turn",
        };
        info!(
            "wolfpack/dump: join_queue sniffer_id={sniffer_id} media={media} pos={queue_position} reason={reason_s}"
        );
    }

    /// `PeerJoinAck` received: encrypted peer slot ready for dump frags.
    pub fn trace_become_peer(sniffer_id: u32, media: &'static str) {
        info!("wolfpack/dump: become_peer sniffer_id={sniffer_id} media={media}");
    }

    /// Leaving control without dump after `join_before_dump` failed (nack or timeout).
    pub fn trace_leave_join_failed(
        sniffer_id: u32,
        media: &'static str,
        join_pressure: bool,
        timed_out: bool,
    ) {
        info!(
            "wolfpack/dump: leave_join_failed sniffer_id={sniffer_id} media={media} join_pressure={join_pressure} timed_out={timed_out}"
        );
    }

    /// Encrypted frag send burst starting.
    pub fn trace_begin_dump(sniffer_id: u32, media: &'static str, frags: u32) {
        info!("wolfpack/dump: begin sniffer_id={sniffer_id} media={media} frags={frags}");
    }

    /// Wardrive dump cycle finished on control channel.
    pub fn trace_end_dump(
        sniffer_id: u32,
        media: &'static str,
        delivered: bool,
        post_dump_done: bool,
    ) {
        info!(
            "wolfpack/dump: end sniffer_id={sniffer_id} media={media} delivered={delivered} post_dump_done={post_dump_done}"
        );
    }

    /// Active dump media label for RX handlers (`wifi` / `ble`).
    static DUMP_MEDIA: AtomicU8 = AtomicU8::new(0);
    const MEDIA_NONE: u8 = 0;
    const MEDIA_WIFI: u8 = 1;
    const MEDIA_BLE: u8 = 2;

    fn set_dump_media(media: &'static str) {
        let tag = match media {
            "wifi" => MEDIA_WIFI,
            "ble" => MEDIA_BLE,
            _ => MEDIA_NONE,
        };
        DUMP_MEDIA.store(tag, Ordering::Relaxed);
    }

    fn dump_media_label() -> &'static str {
        match DUMP_MEDIA.load(Ordering::Relaxed) {
            MEDIA_WIFI => "wifi",
            MEDIA_BLE => "ble",
            _ => "unknown",
        }
    }

    static PEER_READY: AtomicBool = AtomicBool::new(false);
    static PEER_NACK_SEEN: AtomicBool = AtomicBool::new(false);
    static PEER_NACK_RETRY_MS: AtomicU16 = AtomicU16::new(0);
    /// FIFO position from last `PeerJoinNack` (1 = head).
    static PEER_NACK_QUEUE_POSITION: AtomicU8 = AtomicU8::new(1);
    static POST_DUMP_DONE_SEEN: AtomicBool = AtomicBool::new(false);
    static LAST_POST_DUMP_SESSION_SEQ: AtomicU32 = AtomicU32::new(0);
    /// Last broadcast `Ping` peer-pool hints (nack retry backoff only).
    static PEER_SLOTS_FREE: AtomicU8 = AtomicU8::new(0);
    static PEER_JOIN_WAITERS: AtomicU8 = AtomicU8::new(0);

    const PEER_JOIN_ACK_TIMEOUT_MS: u64 = 200;
    const PEER_JOIN_RETRY_MS: u64 = 50;
    /// Short poll while waiting for Ping hint updates (control broadcasts ~300 ms).
    const HINT_POLL_SLEEP_MS: u64 = 20;

    pub fn peer_ready() -> bool {
        PEER_READY.load(Ordering::Relaxed)
    }

    pub fn clear_peer_ready() {
        PEER_READY.store(false, Ordering::Relaxed);
    }

    fn clear_peer_nack() {
        PEER_NACK_SEEN.store(false, Ordering::Relaxed);
    }

    /// Cache peer-pool hints from broadcast `Ping` (wolfPACK).
    pub fn on_control_ping_hints(peer_slots_free: u8, peer_join_waiters: u8) {
        PEER_SLOTS_FREE.store(peer_slots_free, Ordering::Relaxed);
        PEER_JOIN_WAITERS.store(peer_join_waiters, Ordering::Relaxed);
    }

    /// Control unicast `PeerJoinAck`: refresh encrypted peer before dump frags.
    pub fn on_peer_join_ack(sniffer_id: u32, control_mac: [u8; 6], control_ch: u8) {
        let _ = espnow::add_peer(
            control_mac,
            control_ch.max(1),
            sys::wifi_interface_t_WIFI_IF_STA,
        );
        PEER_READY.store(true, Ordering::Relaxed);
        crate::sniffer_link::clear_link_reset_pending();
        trace_become_peer(sniffer_id, dump_media_label());
    }

    /// Broadcast `PeerJoinNack`: leave channel unless bulk fail-safe retry is active.
    pub fn on_peer_join_nack(
        sniffer_id: u32,
        retry_after_ms: u16,
        queue_position: u8,
        reason: PeerJoinNackReason,
    ) {
        PEER_NACK_SEEN.store(true, Ordering::Relaxed);
        PEER_NACK_RETRY_MS.store(retry_after_ms, Ordering::Relaxed);
        // Control uses 1-based FIFO position; default to head when missing.
        PEER_NACK_QUEUE_POSITION.store(queue_position.max(1), Ordering::Relaxed);
        trace_join_queue(sniffer_id, dump_media_label(), queue_position, reason);
    }

    /// Bulk fail-safe: wait for Ping hints before retrying `PeerJoin` (retry_after_ms is a ceiling).
    fn wait_bulk_nack_backoff(sniffer_id: u32, join_deadline_ms: u64, mut drain_rx: impl FnMut()) {
        let position = PEER_NACK_QUEUE_POSITION.load(Ordering::Relaxed);
        let retry_ms = u64::from(PEER_NACK_RETRY_MS.load(Ordering::Relaxed));
        let jitter = u64::from(sniffer_id % 7) * 17;
        let cap_ms = retry_ms.saturating_add(jitter).max(50);
        let wait_until = stats::now_ms().saturating_add(cap_ms).min(join_deadline_ms);

        // Snapshot at nack time; Ping updates while we drain RX.
        let waiters_at_nack = PEER_JOIN_WAITERS.load(Ordering::Relaxed);
        let is_head = position <= 1;

        while stats::now_ms() < wait_until {
            if peer_ready() {
                return;
            }
            drain_rx();

            let slots = PEER_SLOTS_FREE.load(Ordering::Relaxed);
            let waiters = PEER_JOIN_WAITERS.load(Ordering::Relaxed);

            if is_head {
                // FIFO head: retry when control hints a free or LRU-evictable slot.
                if slots > 0 {
                    return;
                }
            } else if waiters < waiters_at_nack {
                // Not head: someone ahead was admitted or pruned.
                return;
            }

            // Starved: poll for broadcast Ping without spamming `PeerJoin`.
            std::thread::sleep(Duration::from_millis(HINT_POLL_SLEEP_MS));
        }
    }

    /// Broadcast `PeerJoin` and drain RX until `PeerJoinAck`, `PeerJoinNack`, or timeout.
    /// Admission is decided by control ack/nack; Ping hints only pace bulk nack retries.
    /// `join_pressure` selects the extended fail-safe window ([`SNIFFER_JOIN_PRESSURE_DEADLINE_MS`]).
    pub fn join_before_dump(
        sniffer_id: u32,
        sniffer_boot_id: u32,
        reason: PeerJoinReason,
        media: &'static str,
        join_pressure: bool,
        mut drain_rx: impl FnMut(),
    ) -> bool {
        set_dump_media(media);
        clear_peer_ready();
        clear_peer_nack();
        let wire = pack_peer_join(PeerJoinV1 {
            sniffer_id,
            sniffer_boot_id,
            join_reason: reason,
        });
        let deadline = if join_pressure {
            stats::now_ms().saturating_add(SNIFFER_JOIN_PRESSURE_DEADLINE_MS)
        } else {
            stats::now_ms().saturating_add(PEER_JOIN_ACK_TIMEOUT_MS)
        };
        let mut last_tx_ms = 0u64;
        while stats::now_ms() < deadline {
            if peer_ready() {
                return true;
            }
            if PEER_NACK_SEEN.load(Ordering::Relaxed) {
                if !join_pressure {
                    trace_leave_join_failed(sniffer_id, media, false, false);
                    return false;
                }
                // Fail-safe: wake early on Ping hints (peer_slots_free / peer_join_waiters).
                wait_bulk_nack_backoff(sniffer_id, deadline, &mut drain_rx);
                clear_peer_nack();
                last_tx_ms = 0;
            }
            let now = stats::now_ms();
            if last_tx_ms == 0 || now.saturating_sub(last_tx_ms) >= PEER_JOIN_RETRY_MS {
                if let Err(e) = espnow::send(espnow::BROADCAST_MAC, &wire) {
                    warn!("sniffer: peer_join broadcast failed: {e:#}");
                }
                last_tx_ms = now;
            }
            drain_rx();
            std::thread::sleep(Duration::from_millis(5));
        }
        let ok = peer_ready();
        if !ok {
            trace_leave_join_failed(sniffer_id, media, join_pressure, true);
        }
        ok
    }

    pub fn clear_post_dump_done_seen() {
        POST_DUMP_DONE_SEEN.store(false, Ordering::Relaxed);
    }

    pub fn post_dump_done_seen() -> bool {
        POST_DUMP_DONE_SEEN.load(Ordering::Relaxed)
    }

    /// Bound sniffer accepted `PostDumpDone`; ignores stale `session_seq`.
    pub fn on_post_dump_done(session_seq: u32) -> bool {
        let prev = LAST_POST_DUMP_SESSION_SEQ.load(Ordering::Relaxed);
        if session_seq < prev {
            return false;
        }
        LAST_POST_DUMP_SESSION_SEQ.store(session_seq, Ordering::Relaxed);
        POST_DUMP_DONE_SEEN.store(true, Ordering::Relaxed);
        true
    }

    /// Drain control RX until deadline, schedule/state change, or `PostDumpDone`.
    /// `step` drains RX and returns `true` to break early (schedule/state change).
    pub fn drain_control_cmd_window(
        deadline_ms: u64,
        mut step: impl FnMut() -> anyhow::Result<bool>,
    ) -> anyhow::Result<()> {
        clear_post_dump_done_seen();
        while stats::now_ms() < deadline_ms {
            if step()? || post_dump_done_seen() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// Home-zone scan-only liveness: request encrypted peer slot (no bulk frags).
    ///
    /// Caller sends `SnifferAck` after a successful return value. `drain_rx` polls
    /// ESP-NOW during `join_before_dump`.
    pub fn run_peer_refresh_join(
        sniffer_id: u32,
        control_ch: u8,
        mut drain_rx: impl FnMut(),
    ) -> bool {
        trace_return_control_for_dump(sniffer_id, "refresh", control_ch, 0, false, false);
        join_before_dump(
            sniffer_id,
            crate::sniffer_wifi::sniffer_boot_id(),
            PeerJoinReason::PeerRefresh,
            "refresh",
            false,
            || drain_rx(),
        )
    }
}

pub use wolfpack::{
    clear_peer_ready, drain_control_cmd_window, join_before_dump, on_control_ping_hints,
    on_peer_join_ack, on_peer_join_nack, on_post_dump_done, post_dump_done_seen,
    run_peer_refresh_join, trace_begin_dump, trace_end_dump, trace_return_control_for_dump,
};

/// Marks exactly one frag per transport burst with [`FRAG_FLAG_TRANSPORT_END`].
pub struct TransportBurst {
    remaining: u32,
}

impl TransportBurst {
    pub fn new(total_sends: u32) -> Self {
        Self {
            remaining: total_sends,
        }
    }

    pub fn flags_for_next_send(&mut self) -> u8 {
        let flags = if self.remaining == 1 {
            wardriver_proto::FRAG_FLAG_TRANSPORT_END
        } else {
            0
        };
        self.remaining = self.remaining.saturating_sub(1);
        flags
    }
}

pub fn dispatch_wifi_dump<F>(probe_frags: u32, wifi_frags: u32, mut send: F) -> bool
where
    F: FnMut(&mut TransportBurst) -> Result<bool>,
{
    let total = probe_frags.saturating_add(wifi_frags);
    if total == 0 {
        return false;
    }
    let mut burst = TransportBurst::new(total);
    match send(&mut burst) {
        Ok(v) => v,
        Err(e) => {
            warn!("sniffer_dispatch: wifi dump send: {e:#}");
            false
        }
    }
}

pub fn dispatch_ble_dump<F>(ble_frags: u32, mut send: F) -> bool
where
    F: FnMut(&mut TransportBurst) -> Result<bool>,
{
    if ble_frags == 0 {
        return false;
    }
    let mut burst = TransportBurst::new(ble_frags);
    match send(&mut burst) {
        Ok(v) => v,
        Err(e) => {
            warn!("sniffer_dispatch: ble dump send: {e:#}");
            false
        }
    }
}
