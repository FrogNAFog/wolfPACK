//! Deep diagnostics counters nested under `/api/stats` → `diagnostics`.

use serde::Serialize;

#[derive(Serialize)]
pub struct DiagnosticsSnapshot {
    pub ingress_unpack_fail_wifi: u64,
    pub ingress_unpack_fail_ble: u64,
    pub sniffer_wardrive_suppressed_skips: u64,
    pub control_stage_ingress: u64,
    pub control_stage_dedup: u64,
    pub control_stage_reasm: u64,
    pub control_stage_row_encode: u64,
    pub control_denylist_lock_slow: u64,
    pub promisc_rx_queue_inflight: u64,
    pub promisc_rx_queue_hwm: u64,
    pub dedup_wifi_size: u64,
    pub dedup_ble_size: u64,
    pub reasm_wifi_size: u64,
    pub reasm_ble_size: u64,
    pub reasm_overflow_evictions: u64,
    pub control_loop_overruns: u64,
    pub control_loop_frames_drained_last: u64,
    pub control_ingest_worker_batches: u64,
    pub detection_alerts_rx: u64,
}
