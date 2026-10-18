//! Control wrapper: wolfPACK peer pool lives in `wardriver-fleet-realtime`.

use std::sync::{Arc, Mutex};

use wardriver_fleet_realtime::ingest::IngestPeerPoolState;
use wardriver_fleet_realtime::peer_pool::{self, PostDumpFlushFn};

use super::espnow_rx;
use super::exclusion;
use super::ingest_shared::IngestSharedState;

pub use wardriver_fleet_realtime::peer_pool::{
    active_count, begin_post_dump_visit, complete_peer_refresh_visit, confirm_peer_join_admitted,
    enqueue_post_dump_flush, ensure_peer, evict_lru_idle, evict_peer, has_pending_post_dump_work,
    join_queue_waiter_count, mark_transport_end, note_bulk_frag, note_peer_join_attempt,
    peer_evict_blocked, peer_slots_free_estimate, request_queue_hint_ping, send_peer_join_nack,
    take_queue_hint_ping_due, tick_evict_pending, tick_join_queue_prune, tick_stale_liveness_evict,
    tick_visit_arm_expiry, try_admit_peer_join, try_finish_post_dump, PeerJoinNackOut,
    PEER_REFRESH_GRACE_MS, POST_DUMP_DONE_GRACE_MS, POST_DUMP_GRACE_MS,
};

/// Handles for [`tick_ingest_post_dump_phase`] on the `espnow-ingest` worker.
pub struct IngestPostDumpDeps<'a> {
    pub cmd_seq: &'a std::sync::atomic::AtomicU32,
    pub control_boot_id: u32,
    pub peer_if: esp_idf_sys::wifi_interface_t,
    pub sniffers: &'a Arc<Mutex<std::collections::HashSet<[u8; 6]>>>,
    pub scanning_allowed: &'a std::sync::atomic::AtomicBool,
}

struct IngestBridge(Arc<Mutex<IngestSharedState>>);

impl IngestPeerPoolState for IngestBridge {
    fn has_incomplete_reasm_for_mac(&self, mac: &[u8; 6]) -> bool {
        self.0
            .lock()
            .ok()
            .map(|g| espnow_rx::has_incomplete_reasm_for_mac(mac, &g.reasm, &g.reasm_ble))
            .unwrap_or(true)
    }

    fn drop_state_for_sniffer(&mut self, sniffer_id: u32) {
        if let Ok(mut g) = self.0.lock() {
            let super::ingest_shared::IngestSharedState {
                ref mut dedup,
                ref mut dedup_ble,
                ref mut dedup_probe,
                ref mut reasm,
                ref mut reasm_ble,
                ref mut reasm_probe,
                ..
            } = &mut *g;
            exclusion::drop_state_for_sniffer(
                sniffer_id,
                dedup,
                dedup_ble,
                dedup_probe,
                reasm,
                reasm_ble,
                reasm_probe,
            );
        }
    }
}

/// Register ingest mutex for peer-pool reasm guards (call once at boot).
pub fn register_ingest(ingest: Arc<Mutex<IngestSharedState>>) {
    let bridge: Arc<Mutex<dyn IngestPeerPoolState>> = Arc::new(Mutex::new(IngestBridge(ingest)));
    peer_pool::register_ingest(bridge);
}

pub fn register_post_dump_flush_hook(f: PostDumpFlushFn) {
    peer_pool::register_post_dump_flush(f);
}

pub fn tick_ingest_post_dump_phase(
    _ingest: &Arc<Mutex<IngestSharedState>>,
    deps: &IngestPostDumpDeps<'_>,
) {
    let inner = wardriver_fleet_realtime::peer_pool::IngestPostDumpDeps {
        cmd_seq: deps.cmd_seq,
        control_boot_id: deps.control_boot_id,
        peer_if: deps.peer_if,
        sniffers: deps.sniffers,
        scanning_allowed: deps.scanning_allowed,
    };
    peer_pool::tick_ingest_post_dump_phase(&inner);
}
