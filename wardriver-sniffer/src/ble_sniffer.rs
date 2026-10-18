//! Passive BLE observations for sniffer **BLE** role.
//!
//! When `ble` is on, this module initializes NimBLE (observer role) and performs a bounded
//! scan window (passive by default; active when control sets `ble_active_scan`). LPR-camera heuristics
//! run inline on every advertisement; qualifying hits abort the scan early. Wardrive bulk rows
//! are collected when enabled.

use wardriver_core::lpr_camera_ble::BleCameraAlert;
use wardriver_proto::{BleEntryV1, RemoteIdAlertV1, ScanEntryExtras};

/// One BLE scan window: optional wardrive rows plus any detection alerts raised during capture.
#[derive(Debug, Default)]
pub struct BleSweepOutcome {
    pub wardrive_entries: Vec<BleEntryV1>,
    pub wardrive_extras: Vec<ScanEntryExtras>,
    pub detection_alerts: Vec<BleCameraAlert>,
    pub remote_id_alerts: Vec<RemoteIdAlertV1>,
    /// True when the scan window ended early because a detection alert was queued.
    #[allow(dead_code)]
    pub aborted_for_detection: bool,
    /// True when the scan window ended early because a Remote ID alert was queued.
    #[allow(dead_code)]
    pub aborted_for_remote_id: bool,
}

#[cfg(feature = "ble")]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use log::{info, warn};

    use super::BleSweepOutcome;
    use wardriver_core::detection::ble_mask_needs_detection;
    use wardriver_core::lpr_camera_ble::{BleCameraAlert, LprCameraBleDetector};
    use wardriver_core::remote_id::RemoteIdDetector;
    use wardriver_proto::{BleEntryV1, RemoteIdAlertV1, ScanEntryExtras};

    struct BleState {
        initialized: AtomicBool,
        synced: AtomicBool,
        own_addr_type: Mutex<Option<u8>>,
        entries: Mutex<HashMap<([u8; 6], u8), (BleEntryV1, ScanEntryExtras)>>,
        detection_sniffer_id: std::sync::atomic::AtomicU32,
        detection_disable_ble_mask: std::sync::atomic::AtomicU32,
        remote_id_disable_mask: std::sync::atomic::AtomicU32,
        /// Precomputed from `detection_disable_ble_mask` at sweep start (skip detection ingest when false).
        detection_enabled: AtomicBool,
        remote_id_detection_enabled: AtomicBool,
        /// When false, advertisements are evaluated for LPR-camera detection only (no wardrive buffer).
        detection_wardrive_enabled: AtomicBool,
        /// Set from GAP callback when the first detection alert is queued; main thread aborts scan.
        detection_abort_scan: AtomicBool,
        remote_id_abort_scan: AtomicBool,
        detection_detector: Mutex<LprCameraBleDetector>,
        remote_id_detector: Mutex<RemoteIdDetector>,
        detection_alerts: Mutex<Vec<BleCameraAlert>>,
        remote_id_alerts: Mutex<Vec<RemoteIdAlertV1>>,
        remote_id_alert_seq: std::sync::atomic::AtomicU32,
        remote_id_boot_id: std::sync::atomic::AtomicU32,
    }

    impl BleState {
        fn new() -> Self {
            Self {
                initialized: AtomicBool::new(false),
                synced: AtomicBool::new(false),
                own_addr_type: Mutex::new(None),
                entries: Mutex::new(HashMap::new()),
                detection_sniffer_id: std::sync::atomic::AtomicU32::new(0),
                detection_disable_ble_mask: std::sync::atomic::AtomicU32::new(0),
                remote_id_disable_mask: std::sync::atomic::AtomicU32::new(0),
                detection_enabled: AtomicBool::new(false),
                remote_id_detection_enabled: AtomicBool::new(false),
                detection_wardrive_enabled: AtomicBool::new(false),
                detection_abort_scan: AtomicBool::new(false),
                remote_id_abort_scan: AtomicBool::new(false),
                detection_detector: Mutex::new(LprCameraBleDetector::new()),
                remote_id_detector: Mutex::new(RemoteIdDetector::new()),
                detection_alerts: Mutex::new(Vec::new()),
                remote_id_alerts: Mutex::new(Vec::new()),
                remote_id_alert_seq: std::sync::atomic::AtomicU32::new(0),
                remote_id_boot_id: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    static STATE: OnceLock<BleState> = OnceLock::new();

    fn state() -> &'static BleState {
        STATE.get_or_init(BleState::new)
    }

    unsafe extern "C" fn on_sync() {
        // Called from NimBLE host task context.
        state().synced.store(true, Ordering::Release);
    }

    const NAME_MAX: usize = 22;
    const MAX_ENTRIES_IN_SWEEP_BUFFER: usize = 512;

    fn remote_id_alert_to_v1(
        a: &wardriver_core::remote_id::RemoteIdAlert,
        seq: u32,
        sniffer_boot_id: u32,
        scan_end_ms: u32,
        scan_end_unix_ms: u64,
    ) -> RemoteIdAlertV1 {
        crate::remote_id_alert_wire::remote_id_alert_to_v1(
            a,
            seq,
            sniffer_boot_id,
            scan_end_ms,
            scan_end_unix_ms,
        )
    }

    fn remote_id_ble_mask_needs_detection(disable_mask: u32) -> bool {
        use wardriver_core::detection::{
            is_remote_id_ble_method_disabled, RemoteIdBleDetectionMethod,
        };
        [
            RemoteIdBleDetectionMethod::ServiceDataFffa,
            RemoteIdBleDetectionMethod::LegacyMfg0002,
        ]
        .iter()
        .any(|m| !is_remote_id_ble_method_disabled(disable_mask, m.as_u8()))
    }

    fn push_adv_report(addr: [u8; 6], addr_type: u8, rssi: i8, adv: &[u8]) {
        // NimBLE provides BLE addresses LSB-first (little-endian). Reverse to big-endian
        // so they are stored consistently with WiFi BSSIDs in the database.
        let mut addr = addr;
        addr.reverse();

        let fields = wardriver_core::ble_adv::parse_ble_adv(adv);
        let mut name_arr = [0u8; NAME_MAX];
        let n = fields.name.as_bytes().len().min(NAME_MAX);
        name_arr[..n].copy_from_slice(&fields.name.as_bytes()[..n]);
        let extras = ScanEntryExtras {
            rcoi_ie_payload: Vec::new(),
            ble_cod: fields.cod,
        };
        let entry = BleEntryV1 {
            addr,
            rssi,
            addr_type,
            company_id: fields.company_id,
            name: name_arr,
            obs_mono_ms: wardriver_stats::now_ms() as u32,
        };

        let st = state();
        let sniffer_id = st.detection_sniffer_id.load(Ordering::Relaxed);
        let disable_mask = st.detection_disable_ble_mask.load(Ordering::Relaxed);
        let remote_id_disable_mask = st.remote_id_disable_mask.load(Ordering::Relaxed);
        let now_ms = wardriver_stats::now_ms();
        if st.detection_enabled.load(Ordering::Relaxed) {
            if let Ok(mut det) = st.detection_detector.lock() {
                if let Some(alert) =
                    det.ingest(sniffer_id, now_ms, &entry, &HashSet::new(), disable_mask)
                {
                    if let Ok(mut alerts) = st.detection_alerts.lock() {
                        alerts.push(alert);
                    }
                    st.detection_abort_scan.store(true, Ordering::Release);
                }
            }
        }

        if st.remote_id_detection_enabled.load(Ordering::Relaxed) {
            if let Ok(mut det) = st.remote_id_detector.lock() {
                if let Some(alert) =
                    det.ingest_ble(sniffer_id, now_ms, &entry, adv, remote_id_disable_mask)
                {
                    let wire = remote_id_alert_to_v1(
                        &alert,
                        st.remote_id_alert_seq.load(Ordering::Relaxed),
                        st.remote_id_boot_id.load(Ordering::Relaxed),
                        now_ms as u32,
                        crate::sniffer_espnow::sniffer_unix_now_ms(),
                    );
                    if let Ok(mut alerts) = st.remote_id_alerts.lock() {
                        alerts.push(wire);
                    }
                    st.remote_id_abort_scan.store(true, Ordering::Release);
                }
            }
        }

        if !st.detection_wardrive_enabled.load(Ordering::Acquire) {
            return;
        }

        if let Ok(mut best) = state().entries.lock() {
            if best.len() >= MAX_ENTRIES_IN_SWEEP_BUFFER && !best.contains_key(&(addr, addr_type)) {
                return;
            }
            match best.entry((addr, addr_type)) {
                std::collections::hash_map::Entry::Occupied(mut occ) => {
                    if entry.rssi > occ.get().0.rssi {
                        occ.insert((entry, extras));
                    }
                }
                std::collections::hash_map::Entry::Vacant(vac) => {
                    vac.insert((entry, extras));
                }
            }
        }
    }

    unsafe extern "C" fn gap_event_cb(
        event: *mut esp_idf_sys::ble_gap_event,
        _arg: *mut core::ffi::c_void,
    ) -> i32 {
        if event.is_null() {
            return 0;
        }
        let ev = &*event;
        let ty = ev.type_ as u32;

        if ty == esp_idf_sys::BLE_GAP_EVENT_EXT_DISC {
            let desc = unsafe { ev.__bindgen_anon_1.ext_disc };
            let mut addr = [0u8; 6];
            addr.copy_from_slice(&desc.addr.val);
            let addr_type: u8 = desc.addr.type_;
            let rssi: i8 = desc.rssi;
            let adv = if desc.data.is_null() || desc.length_data == 0 {
                &[][..]
            } else {
                unsafe { core::slice::from_raw_parts(desc.data, desc.length_data as usize) }
            };
            push_adv_report(addr, addr_type, rssi, adv);
            return 0;
        }

        if ty == esp_idf_sys::BLE_GAP_EVENT_DISC {
            let desc = unsafe { &ev.__bindgen_anon_1.disc };
            let mut addr = [0u8; 6];
            addr.copy_from_slice(&desc.addr.val);
            let addr_type: u8 = desc.addr.type_;
            let rssi: i8 = desc.rssi;
            let adv = unsafe {
                core::slice::from_raw_parts(desc.data as *const u8, desc.length_data as usize)
            };
            push_adv_report(addr, addr_type, rssi, adv);
        }

        0
    }

    fn init_once() -> anyhow::Result<()> {
        let st = state();
        if st.initialized.load(Ordering::Acquire) {
            return Ok(());
        }

        // Set sync callback so we know when scanning can begin.
        crate::ble_stack::set_sync_cb(Some(on_sync));
        crate::ble_stack::init_once();

        st.initialized.store(true, Ordering::Release);
        info!("ble: NimBLE initialized (observer)");
        Ok(())
    }

    fn ensure_own_addr_type() -> anyhow::Result<u8> {
        if let Ok(g) = state().own_addr_type.lock() {
            if let Some(v) = *g {
                return Ok(v);
            }
        }
        // Wait for sync (host ready).
        let deadline = Instant::now() + Duration::from_secs(3);
        while !state().synced.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !state().synced.load(Ordering::Acquire) {
            anyhow::bail!("nimble host not synced");
        }

        let mut own: u8 = 0;
        unsafe {
            let rc = esp_idf_sys::ble_hs_id_infer_auto(0, &mut own);
            if rc != 0 {
                anyhow::bail!("ble_hs_id_infer_auto rc={rc}");
            }
        }
        if let Ok(mut g) = state().own_addr_type.lock() {
            *g = Some(own);
        }
        Ok(own)
    }

    fn run_scan_radio(window_ms: u32, active_scan: bool) -> anyhow::Result<(bool, bool)> {
        let own = ensure_own_addr_type()?;
        state().detection_abort_scan.store(false, Ordering::Release);
        state().remote_id_abort_scan.store(false, Ordering::Release);

        let mut uncoded = esp_idf_sys::ble_gap_ext_disc_params::default();
        // NimBLE: passive=1 → no SCAN_REQ; passive=0 → active scan.
        uncoded.set_passive(u8::from(!active_scan));
        uncoded.itvl = 0x0010;
        uncoded.window = 0x0010;
        let coded = uncoded;
        let dur_10ms = ((window_ms as u32 + 9) / 10).clamp(1, u32::from(u16::MAX)) as u16;

        unsafe {
            let rc = esp_idf_sys::ble_gap_ext_disc(
                own,
                dur_10ms,
                0,
                1,
                // NO_WL_INITA (0x02): no whitelist, but still emit advertising
                // reports for directed (ADV_DIRECT_IND) adverts whose TargetA is
                // not our own address. The default NO_WL (0x00) instructs the
                // controller to suppress those, which silently drops i-PRO
                // bodycams that advertise only as directed adverts aimed at a
                // paired phone — they would then produce neither a row nor an
                // alert (see docs/IPRO_BLE directed-ad investigation).
                esp_idf_sys::BLE_HCI_SCAN_FILT_NO_WL_INITA as u8,
                0,
                &uncoded,
                &coded,
                Some(gap_event_cb),
                core::ptr::null_mut(),
            );
            if rc != 0 {
                anyhow::bail!("ble_gap_ext_disc rc={rc}");
            }
        }

        let total_wait_ms = window_ms as u64 + 25;
        let mut waited_ms = 0u64;
        const BLE_WAIT_CHUNK_MS: u64 = 15;
        let mut aborted_for_detection = false;
        let mut aborted_for_remote_id = false;
        while waited_ms < total_wait_ms {
            if state().detection_abort_scan.load(Ordering::Acquire) {
                aborted_for_detection = true;
                break;
            }
            if state().remote_id_abort_scan.load(Ordering::Acquire) {
                aborted_for_remote_id = true;
                break;
            }
            let step = BLE_WAIT_CHUNK_MS.min(total_wait_ms - waited_ms);
            std::thread::sleep(Duration::from_millis(step));
            waited_ms += step;
            wardriver_shared::task_wdt::reset_if_subscribed();
        }

        unsafe {
            let _ = esp_idf_sys::ble_gap_disc_cancel();
        }
        Ok((aborted_for_detection, aborted_for_remote_id))
    }

    pub fn prune_camera_detector(now_ms: u64) {
        if let Ok(mut det) = state().detection_detector.lock() {
            det.prune(now_ms);
        }
        if let Ok(mut det) = state().remote_id_detector.lock() {
            det.prune(now_ms);
        }
    }

    /// Passive BLE window with inline LPR-camera / Remote ID detection and optional wardrive buffering.
    pub fn sweep_ble_window(
        sniffer_id: u32,
        disable_ble_mask: u32,
        remote_id_disable_ble_mask: u32,
        alert_seq: u32,
        sniffer_boot_id: u32,
        window_ms: u32,
        wardrive_enabled: bool,
        active_scan: bool,
    ) -> BleSweepOutcome {
        if window_ms == 0 {
            return BleSweepOutcome::default();
        }
        if let Err(e) = init_once() {
            warn!("ble: init failed: {e:#}");
            return BleSweepOutcome::default();
        }
        if let Ok(mut alerts) = state().detection_alerts.lock() {
            alerts.clear();
        }
        if let Ok(mut alerts) = state().remote_id_alerts.lock() {
            alerts.clear();
        }
        if let Ok(mut v) = state().entries.lock() {
            v.clear();
        }
        state()
            .detection_sniffer_id
            .store(sniffer_id, Ordering::Relaxed);
        state()
            .detection_disable_ble_mask
            .store(disable_ble_mask, Ordering::Relaxed);
        state()
            .remote_id_disable_mask
            .store(remote_id_disable_ble_mask, Ordering::Relaxed);
        state()
            .remote_id_alert_seq
            .store(alert_seq, Ordering::Relaxed);
        state()
            .remote_id_boot_id
            .store(sniffer_boot_id, Ordering::Relaxed);
        state().detection_enabled.store(
            ble_mask_needs_detection(disable_ble_mask),
            Ordering::Relaxed,
        );
        state().remote_id_detection_enabled.store(
            remote_id_ble_mask_needs_detection(remote_id_disable_ble_mask),
            Ordering::Relaxed,
        );
        state()
            .detection_wardrive_enabled
            .store(wardrive_enabled, Ordering::Release);

        let scan_ok = run_scan_radio(window_ms, active_scan);
        state()
            .detection_wardrive_enabled
            .store(false, Ordering::Release);

        let (aborted_for_detection, aborted_for_remote_id) = match scan_ok {
            Ok(aborted) => aborted,
            Err(e) => {
                warn!("ble: scan window failed: {e:#}");
                (false, false)
            }
        };

        let detection_alerts = state()
            .detection_alerts
            .lock()
            .map(|mut v| core::mem::take(&mut *v))
            .unwrap_or_default();
        let remote_id_alerts = state()
            .remote_id_alerts
            .lock()
            .map(|mut v| core::mem::take(&mut *v))
            .unwrap_or_default();
        let wardrive_entries = if wardrive_enabled {
            state()
                .entries
                .lock()
                .map(|v| {
                    let mut entries = Vec::with_capacity(v.len());
                    let mut extras = Vec::with_capacity(v.len());
                    for (entry, ex) in v.values() {
                        entries.push(*entry);
                        extras.push(ex.clone());
                    }
                    (entries, extras)
                })
                .unwrap_or_default()
        } else {
            (Vec::new(), Vec::new())
        };

        BleSweepOutcome {
            wardrive_entries: wardrive_entries.0,
            wardrive_extras: wardrive_entries.1,
            detection_alerts,
            remote_id_alerts,
            aborted_for_detection,
            aborted_for_remote_id,
        }
    }
}

#[cfg(feature = "ble")]
pub use imp::{prune_camera_detector, sweep_ble_window};

#[cfg(not(feature = "ble"))]
pub fn sweep_ble_window(
    _sniffer_id: u32,
    _disable_ble_mask: u32,
    _remote_id_disable_ble_mask: u32,
    _alert_seq: u32,
    _sniffer_boot_id: u32,
    _window_ms: u32,
    _wardrive_enabled: bool,
    _active_scan: bool,
) -> BleSweepOutcome {
    BleSweepOutcome::default()
}

#[cfg(not(feature = "ble"))]
pub fn prune_camera_detector(_now_ms: u64) {}
