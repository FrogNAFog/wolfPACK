//! Build [`wardriver_stats::DiagnosticsSnapshot`] from live control atomics.

use std::sync::atomic::Ordering;

use wardriver_stats as stats;
use wardriver_stats::DiagnosticsSnapshot;

fn u32_counter(v: u32) -> u64 {
    u64::from(v)
}

pub(crate) fn build() -> DiagnosticsSnapshot {
    DiagnosticsSnapshot {
        ingress_unpack_fail_wifi: u32_counter(
            stats::INGRESS_UNPACK_FAIL_WIFI.load(Ordering::Relaxed),
        ),
        ingress_unpack_fail_ble: u32_counter(
            stats::INGRESS_UNPACK_FAIL_BLE.load(Ordering::Relaxed),
        ),
        sniffer_wardrive_suppressed_skips: u32_counter(
            stats::SNIFFER_WARDRIVE_SUPPRESSED_SKIPS.load(Ordering::Relaxed),
        ),
        control_stage_ingress: u32_counter(stats::CONTROL_STAGE_INGRESS.load(Ordering::Relaxed)),
        control_stage_dedup: u32_counter(stats::CONTROL_STAGE_DEDUP.load(Ordering::Relaxed)),
        control_stage_reasm: u32_counter(stats::CONTROL_STAGE_REASM.load(Ordering::Relaxed)),
        control_stage_row_encode: u32_counter(
            stats::CONTROL_STAGE_ROW_ENCODE.load(Ordering::Relaxed),
        ),
        control_denylist_lock_slow: u32_counter(
            stats::CONTROL_DENYLIST_LOCK_SLOW.load(Ordering::Relaxed),
        ),
        promisc_rx_queue_inflight: u32_counter(
            stats::PROMISC_RX_QUEUE_INFLIGHT.load(Ordering::Relaxed),
        ),
        promisc_rx_queue_hwm: u32_counter(stats::PROMISC_RX_QUEUE_HWM.load(Ordering::Relaxed)),
        dedup_wifi_size: u32_counter(stats::DEDUP_WIFI_SIZE.load(Ordering::Relaxed)),
        dedup_ble_size: u32_counter(stats::DEDUP_BLE_SIZE.load(Ordering::Relaxed)),
        reasm_wifi_size: u32_counter(stats::REASM_WIFI_SIZE.load(Ordering::Relaxed)),
        reasm_ble_size: u32_counter(stats::REASM_BLE_SIZE.load(Ordering::Relaxed)),
        reasm_overflow_evictions: u32_counter(
            stats::REASM_OVERFLOW_EVICTIONS.load(Ordering::Relaxed),
        ),
        control_loop_overruns: u32_counter(stats::CONTROL_LOOP_OVERRUNS.load(Ordering::Relaxed)),
        control_loop_frames_drained_last: u32_counter(
            stats::CONTROL_LOOP_FRAMES_DRAINED_LAST.load(Ordering::Relaxed),
        ),
        control_ingest_worker_batches: u32_counter(
            stats::CONTROL_INGEST_WORKER_BATCHES.load(Ordering::Relaxed),
        ),
        detection_alerts_rx: u32_counter(stats::DETECTION_ALERTS_RX.load(Ordering::Relaxed)),
    }
}
