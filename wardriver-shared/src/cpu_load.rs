//! Control / sniffer CPU load (0–100%, non-idle task time) from FreeRTOS run-time stats.
//!
//! This is **system-wide “busy” time**: roughly `100% − idle-task runtime share` over the sample
//! window (see `wardriver_cpu_stats.c`). WiFi, NimBLE, and the main loop all count as busy.
//! Sustained high values during active sniffing are often **expected** in dense RF; compare against
//! [`wardriver_stats::PROMISC_RX_DROPPED_QUEUE_FULL`] and sniffer dwell instrumentation before assuming
//! a pure regression.
//!
//! C implementation: `wardriver_cpu_load_poll` in `idf_extra/wardriver_tsens/wardriver_cpu_stats.c`.
//! Call [`poll`] ~1 Hz from the control main loop or sniffer superloop (not from ISRs or httpd).

unsafe extern "C" {
    fn wardriver_cpu_load_poll() -> u8;
}

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

static LAST_POLL_MS: Mutex<u64> = Mutex::new(0);
static CPU_LOAD_PCT: AtomicU8 = AtomicU8::new(0);

const POLL_INTERVAL_MS: u64 = 1000;

pub fn init() {}

pub fn poll(now_ms: u64) {
    let mut g = match LAST_POLL_MS.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let last = *g;
    if last != 0 && now_ms.saturating_sub(last) < POLL_INTERVAL_MS {
        return;
    }
    *g = now_ms;
    drop(g);
    // SAFETY: `wardriver_cpu_load_poll` calls `uxTaskGetSystemState` (non-ISR context only).
    let p = unsafe { wardriver_cpu_load_poll() };
    CPU_LOAD_PCT.store(p, Ordering::Relaxed);
}

#[must_use]
pub fn pct() -> u8 {
    CPU_LOAD_PCT.load(Ordering::Relaxed)
}
