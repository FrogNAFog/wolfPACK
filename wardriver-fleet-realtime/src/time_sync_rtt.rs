//! Post-dump RTT TimeSync: unicast Ping/Pong during wolfPACK encrypted visit window.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use esp_idf_sys::wifi_interface_t;
use log::warn;

use wardriver_proto::{
    mono_delta_ms, one_way_ms_from_rtt, pack_time_set, pack_time_sync_ping, rtt_sane,
    time_set_unix_ms, TimeSetV1, TimeSyncPingV1, TimeSyncPongV1,
};
use wardriver_shared::espnow;
use wardriver_stats as stats;

use crate::gateway_downlink::ensure_sniffer_peer_for_unicast;
use crate::peer_pool;

/// Wall-clock trust hooks (control: GPS/SNTP; gateway: phone TimeSync).
pub struct WallClockHooks {
    pub trusted: fn() -> bool,
    pub trusted_since_ms: fn() -> u64,
    pub unix_now_ms: fn() -> Option<u64>,
}

static WALL_CLOCK: OnceLock<WallClockHooks> = OnceLock::new();

/// Register wall-clock providers once at boot.
pub fn register_wall_clock(hooks: WallClockHooks) {
    let _ = WALL_CLOCK.set(hooks);
}

/// Minimum interval between full Ping→Pong→TimeSet cycles per sniffer.
pub const TIME_SYNC_RTT_INTERVAL_MS: u64 = 600_000;

/// Post-dump wait for pong + TimeSet before `PostDumpDone` when RTT was attempted.
pub const TIME_SYNC_FALLBACK_MS: u64 = 250;

static TIME_SYNC_PING_SEQ: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug)]
struct PendingTimeSyncPing {
    mac: [u8; 6],
    sniffer_id: u32,
    seq: u32,
    ctrl_tx_ms: u32,
    ctrl_unix_ms: u64,
}

static PENDING_TIME_SYNC: Mutex<Option<PendingTimeSyncPing>> = Mutex::new(None);

pub fn rtt_sync_due(sniffer_id: u32, now_ms: u64) -> bool {
    let Some(hooks) = WALL_CLOCK.get() else {
        return false;
    };
    if !(hooks.trusted)() {
        return false;
    }
    let last = stats::sniffer_last_rtt_time_set_ms(sniffer_id);
    let trusted_since = (hooks.trusted_since_ms)();
    if trusted_since != 0 && last < trusted_since {
        return true;
    }
    last == 0 || now_ms.saturating_sub(last) >= TIME_SYNC_RTT_INTERVAL_MS
}

/// After post-dump flush: start RTT when due, else skip for this visit.
pub fn maybe_start_post_dump_time_sync(
    mac: [u8; 6],
    sniffer_id: u32,
    now_ms: u64,
    peer_if: wifi_interface_t,
) {
    let Some(hooks) = WALL_CLOCK.get() else {
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    };
    if sniffer_id == 0 {
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    }
    if !(hooks.trusted)() {
        stats::time_sync_rtt_skipped_untrusted();
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    }
    if !rtt_sync_due(sniffer_id, now_ms) {
        stats::time_sync_rtt_skipped_not_due();
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    }
    let Some(ctrl_unix_ms) = (hooks.unix_now_ms)() else {
        warn!("time_sync_rtt: wall clock unavailable; skip RTT for sniffer_id={sniffer_id}");
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    };
    let seq = TIME_SYNC_PING_SEQ.fetch_add(1, Ordering::Relaxed);
    let ctrl_tx_ms = now_ms.min(u64::from(u32::MAX)) as u32;
    let ping = TimeSyncPingV1 { seq, ctrl_tx_ms };
    if let Err(e) = ensure_sniffer_peer_for_unicast(mac, peer_if, "time_sync ping") {
        warn!("time_sync_rtt: {e:#}");
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    }
    let payload = pack_time_sync_ping(ping);
    if let Err(e) = espnow::send(mac, &payload) {
        warn!("time_sync_rtt: ping send sniffer_id={sniffer_id}: {e:#}");
        peer_pool::mark_post_dump_time_sync_skipped(mac);
        return;
    }
    stats::time_sync_ping_tx();
    if let Ok(mut g) = PENDING_TIME_SYNC.lock() {
        *g = Some(PendingTimeSyncPing {
            mac,
            sniffer_id,
            seq,
            ctrl_tx_ms,
            ctrl_unix_ms,
        });
    }
    peer_pool::mark_post_dump_time_sync_attempted(mac, seq);
}

pub fn handle_time_sync_pong(
    peer: [u8; 6],
    pong: TimeSyncPongV1,
    now_ms: u64,
    peer_if: wifi_interface_t,
) {
    stats::time_sync_pong_rx();
    let pending = PENDING_TIME_SYNC.lock().ok().and_then(|mut g| g.take());
    let Some(pending) = pending else {
        return;
    };
    if pending.mac != peer || pending.seq != pong.seq || pending.sniffer_id != pong.sniffer_id {
        stats::time_sync_rtt_reject();
        if let Ok(mut g) = PENDING_TIME_SYNC.lock() {
            *g = Some(pending);
        }
        return;
    }
    let ctrl_rx_ms = now_ms.min(u64::from(u32::MAX)) as u32;
    let rtt_ms = mono_delta_ms(pending.ctrl_tx_ms, ctrl_rx_ms);
    if !rtt_sane(rtt_ms) {
        stats::time_sync_rtt_reject();
        peer_pool::mark_post_dump_time_sync_done(peer);
        return;
    }
    let one_way = one_way_ms_from_rtt(rtt_ms);
    stats::record_sniffer_time_sync_rtt(
        pending.sniffer_id,
        rtt_ms.min(u16::MAX as u32) as u16,
        one_way.min(u16::MAX as u32) as u16,
    );
    let adjusted_unix = time_set_unix_ms(pending.ctrl_unix_ms, rtt_ms);
    let ts = TimeSetV1 {
        seq: pending.seq,
        ctrl_unix_ms: adjusted_unix,
        ctrl_tx_mono_ms32: ctrl_rx_ms,
    };
    if let Err(e) = ensure_sniffer_peer_for_unicast(peer, peer_if, "time_sync timeset") {
        warn!("time_sync_rtt: {e:#}");
        peer_pool::mark_post_dump_time_sync_done(peer);
        return;
    }
    let payload = pack_time_set(ts);
    if espnow::send(peer, &payload).is_ok() {
        if wardriver_proto::wall_unix_plausible(adjusted_unix) {
            stats::set_sniffer_last_rtt_time_set_ms(pending.sniffer_id, now_ms);
        }
    } else {
        warn!(
            "time_sync_rtt: TimeSet send failed sniffer_id={}",
            pending.sniffer_id
        );
    }
    peer_pool::mark_post_dump_time_sync_done(peer);
}

/// True when post-dump finish may proceed (RTT done, skipped, or timed out).
pub fn post_dump_time_sync_ready(
    time_sync_attempted: bool,
    time_sync_done: bool,
    transport_end_ms: u64,
    now_ms: u64,
) -> bool {
    if !time_sync_attempted || time_sync_done {
        return true;
    }
    now_ms >= transport_end_ms.saturating_add(TIME_SYNC_FALLBACK_MS)
}
