//! Minimal NimBLE host bring-up for sniffer BLE observer builds.

#![cfg(feature = "ble")]

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::OnceLock;

use log::{info, warn};

struct BleStackState {
    init_state: AtomicU8, // 0=uninit, 1=initing, 2=ok
}

impl BleStackState {
    fn new() -> Self {
        Self {
            init_state: AtomicU8::new(0),
        }
    }
}

static STATE: OnceLock<BleStackState> = OnceLock::new();
static SYNC_CB_PTR: AtomicUsize = AtomicUsize::new(0);

fn state() -> &'static BleStackState {
    STATE.get_or_init(BleStackState::new)
}

unsafe extern "C" fn host_task(_arg: *mut core::ffi::c_void) {
    esp_idf_sys::nimble_port_run();
    esp_idf_sys::nimble_port_freertos_deinit();
}

pub type SyncCb = unsafe extern "C" fn();

/// Set NimBLE `ble_hs_cfg.sync_cb` to be used on the *next* init.
pub fn set_sync_cb(cb: Option<SyncCb>) {
    let st = state();
    if st.init_state.load(Ordering::Acquire) == 2 {
        return;
    }
    let v = cb.map(|f| f as usize).unwrap_or(0);
    SYNC_CB_PTR.store(v, Ordering::Release);
}

/// Initialize NimBLE host once (best-effort; safe under concurrency).
pub fn init_once() {
    let st = state();
    if st.init_state.load(Ordering::Acquire) == 2 {
        return;
    }

    if st
        .init_state
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }

    let mut ok = false;
    unsafe {
        let cb_ptr = SYNC_CB_PTR.load(Ordering::Acquire);
        if cb_ptr != 0 {
            let cb: SyncCb = core::mem::transmute::<usize, SyncCb>(cb_ptr);
            esp_idf_sys::ble_hs_cfg.sync_cb = Some(cb);
        }

        let rc = esp_idf_sys::nimble_port_init();
        if rc != 0 {
            warn!("ble: nimble_port_init rc={rc}");
        } else {
            esp_idf_sys::nimble_port_freertos_init(Some(host_task));
            ok = true;
        }
    }

    if ok {
        st.init_state.store(2, Ordering::Release);
        info!("ble: NimBLE host initialized");
    } else {
        st.init_state.store(0, Ordering::Release);
    }
}
