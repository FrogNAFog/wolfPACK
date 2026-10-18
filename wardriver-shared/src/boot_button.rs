//! XIAO BOOT button long-press detector for fleet unbind (all firmware roles).
//!
//! A dedicated pthread polls GPIO so main loops never block on debounce/hold timing.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_hal::gpio::{Input, InputPin, PinDriver, Pull};
use log::{info, warn};
use wardriver_core::boot_button_hold::{hold_complete, FLEET_UNBIND_HOLD_MS};

use crate::spiram_thread;
use crate::task_wdt;

/// Poll interval for BOOT sampling (ms).
const POLL_MS: u64 = 40;

/// Ignore bounce shorter than this while pressed (ms).
const DEBOUNCE_MS: u64 = 80;

/// Button worker stack — GPIO poll only.
const WORKER_STACK_BYTES: usize = 6_144;

static FLEET_UNBIND_REQUESTED: AtomicBool = AtomicBool::new(false);
static HOLD_ELAPSED_MS: AtomicU32 = AtomicU32::new(0);

/// Take a pending fleet-unbind request (clears the flag).
#[must_use]
pub fn take_fleet_unbind_request() -> bool {
    FLEET_UNBIND_REQUESTED.swap(false, Ordering::AcqRel)
}

/// Elapsed hold time while BOOT is currently pressed (0 when released).
#[must_use]
pub fn hold_elapsed_ms() -> u64 {
    u64::from(HOLD_ELAPSED_MS.load(Ordering::Relaxed))
}

/// Handle to the BOOT button worker thread.
pub struct BootButtonHandle {
    _worker: JoinHandle<()>,
}

impl BootButtonHandle {
    /// Start polling `pin` (active-low with internal pull-up).
    pub fn spawn(pin: impl InputPin + 'static) -> Result<Self> {
        let worker = spiram_thread::spawn_named("boot-button", WORKER_STACK_BYTES, move || {
            let Ok(mut driver) = PinDriver::input(pin, Pull::Up) else {
                warn!("boot_button: gpio input failed");
                return;
            };
            boot_button_worker(&mut driver);
        })
        .context("boot_button: spawn worker")?;
        info!(
            "boot_button: worker started (hold_ms={})",
            FLEET_UNBIND_HOLD_MS
        );
        Ok(Self { _worker: worker })
    }
}

fn boot_button_worker(pin: &mut PinDriver<'static, Input>) {
    let mut pressed_ms: u64 = 0;
    let mut armed = true;
    let mut stable_pressed = false;

    loop {
        task_wdt::reset_if_subscribed();
        let raw_pressed = pin.is_low();
        if raw_pressed {
            if !stable_pressed {
                if pressed_ms == 0 {
                    pressed_ms = POLL_MS;
                } else {
                    pressed_ms = pressed_ms.saturating_add(POLL_MS);
                }
                if pressed_ms >= DEBOUNCE_MS {
                    stable_pressed = true;
                }
            } else {
                pressed_ms = pressed_ms.saturating_add(POLL_MS);
            }
        } else {
            pressed_ms = 0;
            stable_pressed = false;
            armed = true;
        }

        if stable_pressed {
            HOLD_ELAPSED_MS.store(
                pressed_ms.min(u64::from(u32::MAX)) as u32,
                Ordering::Relaxed,
            );
            if armed && hold_complete(pressed_ms, FLEET_UNBIND_HOLD_MS) {
                FLEET_UNBIND_REQUESTED.store(true, Ordering::Release);
                armed = false;
                info!("boot_button: fleet unbind requested (hold complete)");
            }
        } else {
            HOLD_ELAPSED_MS.store(0, Ordering::Relaxed);
        }

        task_wdt::sleep_feeding_wdt(Duration::from_millis(POLL_MS));
    }
}

/// Optional progress hook for OLED countdown on control (`board-xiao`).
pub fn format_unbind_countdown(elapsed_ms: u64) -> Option<String> {
    use wardriver_core::boot_button_hold::hold_seconds_remaining;
    let rem = hold_seconds_remaining(elapsed_ms, FLEET_UNBIND_HOLD_MS)?;
    Some(format!("UNBIND {rem}s"))
}

/// Log a countdown line when hold is in progress (Marauder / gateway without OLED).
pub fn log_hold_progress_if_due(last_logged_sec: &mut Option<u64>, elapsed_ms: u64) {
    use wardriver_core::boot_button_hold::hold_seconds_remaining;
    let Some(rem) = hold_seconds_remaining(elapsed_ms, FLEET_UNBIND_HOLD_MS) else {
        return;
    };
    if last_logged_sec == &Some(rem) {
        return;
    }
    *last_logged_sec = Some(rem);
    warn!("boot_button: hold {rem}s more for fleet unbind");
}
