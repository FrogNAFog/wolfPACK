//! FreeRTOS task watchdog (TWDT): register the current task and feed periodically.
//!
//! Requires `CONFIG_ESP_TASK_WDT_EN=y` and `CONFIG_ESP_TASK_WDT_INIT=y` in sdkconfig.
//! See ESP-IDF `esp_task_wdt.h`.
//!
//! **Timeouts use [`wardriver_stats::now_ms`] (`esp_timer`), not `std::time::Instant`.**
//! On ESP-IDF pthread builds `Instant::now()` can stall, which would hang any loop that
//! uses it for deadlines (boot sd-io wait, RPC poll slices, etc.).

use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_sys as sys;

use wardriver_stats as stats;

/// Poll interval while blocking on I/O or sleep so subscribed tasks feed TWDT.
const WDT_FEED_SLICE_MS: u64 = 500;
/// [`Receiver::try_recv`] poll period on ESP-IDF (pthread `recv_timeout` can miss wakeups).
const RECV_POLL_SLICE_MS: u64 = 10;

/// Subscribe the **current** FreeRTOS task to the task watchdog.
///
/// Safe to call more than once: `ESP_ERR_INVALID_STATE` means already registered.
pub fn register_current_task_for_watchdog() {
    unsafe {
        let h = sys::xTaskGetCurrentTaskHandle();
        let rc = sys::esp_task_wdt_add(h);
        if rc != 0 && rc != sys::ESP_ERR_INVALID_STATE as i32 {
            log::warn!("task_wdt: esp_task_wdt_add rc={rc}");
        }
    }
}

/// Reset the task watchdog for the current task (no-op if not subscribed).
#[inline]
pub fn reset_if_subscribed() {
    unsafe {
        let h = sys::xTaskGetCurrentTaskHandle();
        if sys::esp_task_wdt_status(h) != sys::ESP_OK as i32 {
            return;
        }
        let rc = sys::esp_task_wdt_reset();
        if rc != 0 {
            log::warn!("task_wdt: esp_task_wdt_reset rc={rc}");
        }
    }
}

/// [`Receiver::recv_timeout`] in slices, feeding TWDT between attempts.
pub fn recv_timeout_feeding_wdt<T>(
    rx: &Receiver<T>,
    timeout: Duration,
) -> Result<T, RecvTimeoutError> {
    let deadline_ms = stats::now_ms() + timeout.as_millis() as u64;
    loop {
        reset_if_subscribed();
        let now = stats::now_ms();
        if now >= deadline_ms {
            return Err(RecvTimeoutError::Timeout);
        }
        let slice_ms = (deadline_ms - now).min(WDT_FEED_SLICE_MS);
        match rx.recv_timeout(Duration::from_millis(slice_ms)) {
            Ok(v) => return Ok(v),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Poll [`Receiver::try_recv`] until a message arrives or `timeout` elapses.
///
/// ESP-IDF pthread builds can fail to wake [`Receiver::recv_timeout`] when another
/// thread calls `Sender::send`; polling avoids that hang while still feeding TWDT.
pub fn recv_polling_feeding_wdt<T>(
    rx: &Receiver<T>,
    timeout: Duration,
) -> Result<T, RecvTimeoutError> {
    let deadline_ms = stats::now_ms() + timeout.as_millis() as u64;
    loop {
        reset_if_subscribed();
        match rx.try_recv() {
            Ok(v) => return Ok(v),
            Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
            Err(TryRecvError::Empty) => {
                if stats::now_ms() >= deadline_ms {
                    return Err(RecvTimeoutError::Timeout);
                }
                sleep_feeding_wdt(Duration::from_millis(RECV_POLL_SLICE_MS));
            }
        }
    }
}

/// FreeRTOS `vTaskDelay` in slices, feeding TWDT between delays.
pub fn sleep_feeding_wdt(total: Duration) {
    let total_ms = total.as_millis() as u64;
    if total_ms == 0 {
        return;
    }
    let deadline_ms = stats::now_ms().saturating_add(total_ms);
    while stats::now_ms() < deadline_ms {
        reset_if_subscribed();
        let remaining_ms = deadline_ms
            .saturating_sub(stats::now_ms())
            .min(WDT_FEED_SLICE_MS);
        if remaining_ms == 0 {
            break;
        }
        FreeRtos::delay_ms(remaining_ms as u32);
    }
}
