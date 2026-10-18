//! Background workers: ESP-NOW ingest.

use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_sys::wifi_interface_t;
use log::{info, warn};

use crate::config;
use crate::gps;
use crate::storage;
use wardriver_shared::espnow;
use wardriver_shared::memory_policy::RuntimeMemoryPolicy;
use wardriver_shared::spiram_queue::{self, RecvTimeoutError, TryRecvError};
use wardriver_shared::spiram_thread;
use wardriver_stats as stats;

use super::dedup_maintenance::{maybe_trim_mac_table, prune_recent_maps, prune_recent_maps_ble};
use super::ingest_shared::IngestSharedState;
use super::ingress::{self, ControlIngressCtx};

/// Immutable handles shared by the ingest worker thread.
pub(crate) struct IngestWorkerDeps {
    pub control_boot_id: u32,
    pub mem_policy: RuntimeMemoryPolicy,
    pub peer_if: wifi_interface_t,
    pub own_management_ap_bssid: Option<[u8; 6]>,
    pub sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    pub nvs: Arc<EspNvsPartition<NvsDefault>>,
    pub cfg: Arc<config::Config>,
    pub scanning_allowed: Arc<AtomicBool>,
    pub cmd_seq: Arc<AtomicU32>,
    pub wigle_home: Arc<Mutex<Option<[u8; 6]>>>,
    pub wigle_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub wigle_ssids_manual: Arc<RwLock<HashSet<String>>>,
    pub detection_ignore: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    pub gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    pub alert_hub: Arc<crate::alert_feedback::AlertHub>,
    pub sd_io: Arc<storage::SdIoHandle>,
}

pub(crate) struct ControlWorkers {
    _ingest_thread: thread::JoinHandle<()>,
}

/// Spawn ESP-NOW ingest worker thread (`rx_rx` is the SPIRAM ESP-NOW queue).
pub(crate) fn spawn_control_workers(
    rx_rx: spiram_queue::Receiver<espnow::RxFrame>,
    ingest: Arc<Mutex<IngestSharedState>>,
    deps: Arc<IngestWorkerDeps>,
) -> Result<ControlWorkers> {
    info!(
        "control/workers: spawn (internal_free={} spiram_free={} slice={})",
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
        deps.mem_policy.control_loop_slice_frames,
    );

    // Hold rx_rx outside the spawn closure so a failed `thread::spawn` does not drop the receiver.
    let rx_slot = Arc::new(Mutex::new(Some(rx_rx)));
    let ingest_shared = Arc::clone(&ingest);
    let ingest_deps = Arc::clone(&deps);

    let mut last_err: Option<std::io::Error> = None;
    for &stack_bytes in spiram_thread::INGEST_STACK_FALLBACKS {
        let rx_slot = Arc::clone(&rx_slot);
        let ingest_shared = Arc::clone(&ingest_shared);
        let ingest_deps = Arc::clone(&ingest_deps);
        match spiram_thread::spawn_named("espnow-ingest", stack_bytes, move || {
            let rx_rx = rx_slot
                .lock()
                .ok()
                .and_then(|mut g| g.take())
                .expect("espnow-ingest: rx_rx slot empty");
            ingest_worker_task(rx_rx, ingest_shared, ingest_deps);
        }) {
            Ok(ingest_thread) => {
                stats::CONTROL_INGEST_STACK_BYTES.store(stack_bytes as u32, Ordering::Relaxed);
                info!("control/workers: espnow-ingest spawned stack_bytes={stack_bytes}");
                return Ok(ControlWorkers {
                    _ingest_thread: ingest_thread,
                });
            }
            Err(e) => {
                warn!("control/workers: espnow-ingest spawn failed stack_bytes={stack_bytes}: {e}");
                last_err = Some(e);
            }
        }
    }

    Err(anyhow!(
        "spawn espnow-ingest thread: all stack sizes failed: {:#}",
        last_err.unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "unknown"))
    ))
}

/// Drain up to `slice` frames from the ESP-NOW RX queue. Returns `false` if disconnected.
fn fill_ingest_batch(
    rx_rx: &spiram_queue::Receiver<espnow::RxFrame>,
    batch: &mut Vec<espnow::RxFrame>,
    slice: usize,
) -> bool {
    while batch.len() < slice {
        match rx_rx.try_recv() {
            Ok(frame) => batch.push(frame),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
    true
}

fn ingest_idle_timeout_ms() -> u64 {
    if super::peer_pool::has_pending_post_dump_work() {
        return 5;
    }
    25
}

fn tick_ingest_post_dump_from_deps(
    ingest: &Arc<Mutex<IngestSharedState>>,
    deps: &IngestWorkerDeps,
) {
    let post_dump_deps = super::peer_pool::IngestPostDumpDeps {
        cmd_seq: deps.cmd_seq.as_ref(),
        control_boot_id: deps.control_boot_id,
        peer_if: deps.peer_if,
        sniffers: &deps.sniffers,
        scanning_allowed: &deps.scanning_allowed,
    };
    super::peer_pool::tick_ingest_post_dump_phase(ingest, &post_dump_deps);
}

fn ingest_worker_task(
    rx_rx: spiram_queue::Receiver<espnow::RxFrame>,
    ingest: Arc<Mutex<IngestSharedState>>,
    deps: Arc<IngestWorkerDeps>,
) {
    wardriver_shared::task_wdt::register_current_task_for_watchdog();
    let mut pending: Option<espnow::RxFrame> = None;
    loop {
        wardriver_shared::task_wdt::reset_if_subscribed();
        let slice = deps.mem_policy.control_loop_slice_frames.max(1);
        let mut batch = Vec::with_capacity(slice.min(64));

        if let Some(frame) = pending.take() {
            batch.push(frame);
        }

        if !fill_ingest_batch(&rx_rx, &mut batch, slice) {
            break;
        }

        if batch.is_empty() {
            match rx_rx.recv_timeout(Duration::from_millis(ingest_idle_timeout_ms())) {
                Ok(frame) => batch.push(frame),
                Err(RecvTimeoutError::Timeout) => {
                    tick_ingest_post_dump_from_deps(&ingest, &deps);
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if !fill_ingest_batch(&rx_rx, &mut batch, slice) && batch.is_empty() {
                break;
            }
        }

        let batch_full = batch.len() >= slice;
        {
            let Ok(mut shared) = ingest.lock() else {
                warn!("control/ingest-worker: ingest mutex poisoned");
                continue;
            };
            run_ingest_tick(&mut shared, &deps, batch);
        }
        tick_ingest_post_dump_from_deps(&ingest, &deps);

        if batch_full {
            continue;
        }
        match rx_rx.try_recv() {
            Ok(frame) => {
                pending = Some(frame);
                continue;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => break,
        }
    }
}

fn run_ingest_tick(
    shared: &mut IngestSharedState,
    deps: &IngestWorkerDeps,
    frames: Vec<espnow::RxFrame>,
) {
    let now_ms = stats::now_ms();
    shared.lpr_camera_ble.prune(now_ms);

    let detection_dis_w = stats::detection_disable_wifi_mask();
    let detection_dis_b = stats::detection_disable_ble_mask();
    let remote_id_dis = stats::remote_id_disable_mask();
    tick_detection_enabled(detection_dis_w, detection_dis_b);

    // Clone per batch so GPS writer and main-loop prune do not block on long ingress.
    let gps_hist = deps
        .gps_hist
        .read()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    if !frames.is_empty() {
        stats::CONTROL_INGEST_WORKER_BATCHES.fetch_add(1, Ordering::Relaxed);
        stats::CONTROL_INGEST_FRAMES_DRAINED.fetch_add(frames.len() as u32, Ordering::Relaxed);
        stats::CONTROL_LOOP_FRAMES_DRAINED_LAST.store(frames.len() as u32, Ordering::Relaxed);
        let mut ingress_ctx = ControlIngressCtx {
            now_ms,
            detection_dis_w,
            detection_dis_b,
            remote_id_dis,
            control_boot_id: deps.control_boot_id,
            scanning_allowed: &deps.scanning_allowed,
            cmd_seq: &deps.cmd_seq,
            sniffers: &deps.sniffers,
            nvs: deps.nvs.as_ref(),
            cfg: deps.cfg.as_ref(),
            last_sniffer_boot_id: &mut shared.last_sniffer_boot_id,
            dedup: &mut shared.dedup,
            dedup_ble: &mut shared.dedup_ble,
            dedup_probe: &mut shared.dedup_probe,
            reasm: &mut shared.reasm,
            reasm_ble: &mut shared.reasm_ble,
            reasm_probe: &mut shared.reasm_probe,
            mem_policy: &deps.mem_policy,
            own_management_ap_bssid: deps.own_management_ap_bssid,
            wigle_home: &deps.wigle_home,
            wigle_manual: &deps.wigle_manual,
            wigle_ssids_manual: &deps.wigle_ssids_manual,
            detection_ignore: &deps.detection_ignore,
            gps_hist: &gps_hist,
            gps_fix: &deps.gps_fix,
            sd_io: deps.sd_io.as_ref(),
            mac_table: &mut shared.mac_table,
            lpr_camera_ble: &mut shared.lpr_camera_ble,
            detection_alert_log_wifi: &mut shared.detection_alert_log_wifi,
            detection_alert_log_ble: &mut shared.detection_alert_log_ble,
            remote_id_alert_log: &mut shared.remote_id_alert_log,
            alert_hub: &deps.alert_hub,
            detection_last_alert: &mut shared.detection_last_alert,
        };
        ingress::process_espnow_frames(&mut ingress_ctx, frames);
        prune_recent_maps(
            &mut shared.dedup,
            &mut shared.reasm,
            now_ms,
            &deps.mem_policy,
        );
        prune_recent_maps_ble(
            &mut shared.dedup_ble,
            &mut shared.reasm_ble,
            now_ms,
            &deps.mem_policy,
        );
        maybe_trim_mac_table(&mut shared.mac_table, &deps.mem_policy);
    }
    super::ingest_shared::publish_ingest_gauges(shared);
    crate::recon::poll_session_end(deps.sd_io.as_ref(), &deps.sniffers);
}

fn tick_detection_enabled(detection_dis_w: u32, detection_dis_b: u32) {
    const WIFI_METHOD_MASK: u32 = 0b111;
    const BLE_METHOD_MASK: u32 = 0b1111;
    let enabled = (detection_dis_w & WIFI_METHOD_MASK) != WIFI_METHOD_MASK
        || (detection_dis_b & BLE_METHOD_MASK) != BLE_METHOD_MASK;
    stats::set_detection_enabled(
        enabled,
        if enabled {
            stats::DETECTION_DISABLED_NONE
        } else {
            stats::DETECTION_DISABLED_ALL_METHODS_MASKED
        },
    );
}
