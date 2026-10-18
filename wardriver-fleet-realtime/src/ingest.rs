//! Ingest mutex hooks for peer-pool eviction guards (dedup / reasm in-flight).

use std::sync::{Arc, Mutex, OnceLock};

/// Dedup / reasm state the peer pool consults before LRU or grace eviction.
pub trait IngestPeerPoolState: Send {
    /// True when `mac`'s sniffer still has in-flight wifi or BLE reassembly.
    fn has_incomplete_reasm_for_mac(&self, mac: &[u8; 6]) -> bool;
    /// Drop dedup + reasm rows after sniffer reboot or stale liveness evict.
    fn drop_state_for_sniffer(&mut self, sniffer_id: u32);
}

static INGEST_FOR_PEER_POOL: OnceLock<Arc<Mutex<dyn IngestPeerPoolState>>> = OnceLock::new();

/// Register shared ingest maps (called once at boot).
pub fn register_ingest(ingest: Arc<Mutex<dyn IngestPeerPoolState>>) {
    let _ = INGEST_FOR_PEER_POOL.set(ingest);
}

pub(crate) fn with_ingest<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&dyn IngestPeerPoolState) -> R,
{
    INGEST_FOR_PEER_POOL
        .get()
        .and_then(|ing| ing.lock().ok().map(|g| f(&*g)))
}

/// Non-blocking ingest access (returns `None` when the mutex is already held).
pub(crate) fn try_with_ingest_mut<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut dyn IngestPeerPoolState) -> R,
{
    INGEST_FOR_PEER_POOL
        .get()
        .and_then(|ing| ing.try_lock().ok().map(|mut g| f(&mut *g)))
}
