//! XIAO ESP32-C5 user LED (GPIO27) health feedback for sniffer firmware.
//!
//! A dedicated pthread drives the LED so patterns stay smooth while the main loop
//! blocks on RF dwell / BLE scan / dump I/O. **Healthy** uses LEDC hardware fade
//! (inhale/exhale); fault patterns are software-timed with phase catch-up.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_hal::gpio::OutputPin;
use esp_idf_hal::ledc::{
    config::TimerConfig, LedcChannel, LedcDriver, LedcTimer, LedcTimerDriver, LowSpeed,
};
use esp_idf_hal::units::Hertz;
use log::warn;

use crate::sniffer_config::SnifferConfig;
use wardriver_shared::spiram_thread;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

/// Seeed XIAO ESP32-C5 yellow user LED (`LED_BUILTIN`).
pub const XIAO_USER_LED_GPIO: u8 = 27;

/// Grace after boot or link recovery before "no control link" is shown.
pub const BOOT_RECONNECT_GRACE_MS: u64 = 30_000;

/// No ESP-NOW from control for this long ⇒ control stale (matches link watchdog recover).
pub const CONTROL_STALE_MS: u64 = 20_000;

/// Full inhale + exhale for healthy breathing (ms); each half uses hardware fade.
pub const HEALTHY_BREATH_PERIOD_MS: u64 = 4_000;

/// Fault-pattern worker poll interval (ms).
const LED_TASK_POLL_MS: u64 = 40;

/// LED worker stack — no RF/SD work; small internal stack is enough.
const LED_WORKER_STACK_BYTES: usize = 6_144;

/// XIAO user LED is active-low (GPIO low = lit).
const LED_ACTIVE_HIGH: bool = false;

/// LEDC carrier for fault patterns and fade base rate.
const LEDC_PWM_HZ: u32 = 1_000;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedHealth {
    Healthy = 0,
    NotAdopted = 1,
    NoControlLink = 2,
    ControlStale = 3,
    BootReconnecting = 4,
}

impl LedHealth {
    fn from_wire(v: u8) -> Self {
        match v {
            0 => Self::Healthy,
            1 => Self::NotAdopted,
            2 => Self::NoControlLink,
            3 => Self::ControlStale,
            4 => Self::BootReconnecting,
            _ => Self::Healthy,
        }
    }
}

#[derive(Clone, Copy)]
struct Phase {
    on: bool,
    duration_ms: u64,
}

fn phases_for(state: LedHealth) -> &'static [Phase] {
    match state {
        LedHealth::Healthy => &[],
        LedHealth::ControlStale => &[Phase {
            on: true,
            duration_ms: u64::MAX,
        }],
        LedHealth::NotAdopted => &[
            Phase {
                on: true,
                duration_ms: 200,
            },
            Phase {
                on: false,
                duration_ms: 200,
            },
        ],
        LedHealth::NoControlLink => &[
            Phase {
                on: true,
                duration_ms: 150,
            },
            Phase {
                on: false,
                duration_ms: 150,
            },
            Phase {
                on: true,
                duration_ms: 150,
            },
            Phase {
                on: false,
                duration_ms: 900,
            },
        ],
        LedHealth::BootReconnecting => &[
            Phase {
                on: true,
                duration_ms: 500,
            },
            Phase {
                on: false,
                duration_ms: 500,
            },
            Phase {
                on: true,
                duration_ms: 150,
            },
            Phase {
                on: false,
                duration_ms: 150,
            },
            Phase {
                on: true,
                duration_ms: 150,
            },
            Phase {
                on: false,
                duration_ms: 900,
            },
        ],
    }
}

/// Derive LED health from sniffer link state (health only — not job / scan state).
#[must_use]
pub fn evaluate_health(
    cfg: &SnifferConfig,
    control_established: bool,
    last_control_rx_ms: Option<u64>,
    reconnect_grace_until_ms: u64,
    now_ms: u64,
) -> LedHealth {
    if !cfg.is_fleet_bound() {
        return LedHealth::NotAdopted;
    }
    if now_ms < reconnect_grace_until_ms {
        return LedHealth::BootReconnecting;
    }
    if let Some(t) = last_control_rx_ms {
        if now_ms.saturating_sub(t) >= CONTROL_STALE_MS {
            return LedHealth::ControlStale;
        }
    }
    if !control_established {
        return LedHealth::NoControlLink;
    }
    LedHealth::Healthy
}

/// Main-loop handle: publishes desired health; worker owns GPIO/LEDC.
pub struct SnifferLedHandle {
    health: Arc<AtomicU8>,
    _worker: JoinHandle<()>,
}

impl SnifferLedHandle {
    pub fn spawn<C, T>(channel: C, timer: T, pin: impl OutputPin + 'static) -> Result<Self>
    where
        C: LedcChannel<SpeedMode = LowSpeed> + 'static,
        T: LedcTimer<SpeedMode = LowSpeed> + 'static,
    {
        let led = SnifferLed::new(channel, timer, pin)?;
        let health = Arc::new(AtomicU8::new(LedHealth::Healthy as u8));
        let health_worker = Arc::clone(&health);
        let worker = spiram_thread::spawn_named("sniffer-led", LED_WORKER_STACK_BYTES, move || {
            sniffer_led_worker(health_worker, led);
        })
        .context("sniffer_led: spawn worker")?;
        Ok(Self {
            health,
            _worker: worker,
        })
    }

    /// Publish desired health (lock-free); worker applies on its next wake.
    pub fn set_health(&self, health: LedHealth) {
        self.health.store(health as u8, Ordering::Release);
    }
}

/// LEDC driver + fault-pattern phase state (worker thread only).
struct SnifferLed {
    channel: LedcDriver<'static>,
    max_duty: u32,
    /// Pattern currently rendered (differs from desired while a fade is finishing).
    rendered: LedHealth,
}

impl SnifferLed {
    fn new<C, T>(channel: C, timer: T, pin: impl OutputPin + 'static) -> Result<Self>
    where
        C: LedcChannel<SpeedMode = LowSpeed> + 'static,
        T: LedcTimer<SpeedMode = LowSpeed> + 'static,
    {
        let timer_cfg = TimerConfig::new().frequency(Hertz(LEDC_PWM_HZ));
        let timer = LedcTimerDriver::new(timer, &timer_cfg).context("sniffer_led: LEDC timer")?;
        let mut channel =
            LedcDriver::new(channel, timer, pin).context("sniffer_led: LEDC channel")?;
        let max_duty = channel.get_max_duty();
        channel
            .set_duty(map_duty(0, max_duty))
            .context("sniffer_led: duty off")?;
        Ok(Self {
            channel,
            max_duty,
            rendered: LedHealth::Healthy,
        })
    }

    /// Stop hardware fade and snap to off before fault patterns take over.
    fn leave_healthy(&mut self) {
        let _ = self.channel.fade_stop();
        let _ = self.channel.set_duty(map_duty(0, self.max_duty));
    }

    /// One inhale or exhale via LEDC hardware fade; returns false if health left Healthy.
    fn fade_half_interruptible(
        &mut self,
        health: &AtomicU8,
        logical_target: u32,
        fade_ms: i32,
    ) -> bool {
        if health.load(Ordering::Acquire) != LedHealth::Healthy as u8 {
            return false;
        }
        let target = map_duty(logical_target.min(self.max_duty), self.max_duty);
        if let Err(e) = self.channel.fade_with_time(target, fade_ms, false) {
            warn!("sniffer_led: fade_with_time: {e}");
            return false;
        }
        let deadline = stats::now_ms().saturating_add(fade_ms.max(0) as u64);
        while stats::now_ms() < deadline {
            if health.load(Ordering::Acquire) != LedHealth::Healthy as u8 {
                let _ = self.channel.fade_stop();
                return false;
            }
            task_wdt::reset_if_subscribed();
            task_wdt::sleep_feeding_wdt(Duration::from_millis(LED_TASK_POLL_MS));
        }
        true
    }

    /// Hardware breathing loop body: fade up then fade down (half period each).
    fn breathe_once(&mut self, health: &AtomicU8) -> bool {
        let half_ms = (HEALTHY_BREATH_PERIOD_MS / 2).max(1) as i32;
        let max = self.max_duty;
        if !self.fade_half_interruptible(health, max, half_ms) {
            return false;
        }
        self.fade_half_interruptible(health, 0, half_ms)
    }

    fn tick_fault(&mut self, health: LedHealth, now_ms: u64) {
        if health != self.rendered {
            self.rendered = health;
        }

        if health == LedHealth::ControlStale {
            let _ = self
                .channel
                .set_duty(map_duty(self.max_duty, self.max_duty));
            return;
        }

        let phases = phases_for(health);
        if phases.is_empty() {
            return;
        }

        // Sync phase to wall clock so a long main-loop stall does not stretch one blink step.
        let cycle_ms: u64 = phases.iter().map(|p| p.duration_ms).sum();
        if cycle_ms == 0 {
            return;
        }
        let t = now_ms % cycle_ms;
        let mut acc: u64 = 0;
        for phase in phases {
            let end = acc.saturating_add(phase.duration_ms);
            if t < end {
                let logical = if phase.on { self.max_duty } else { 0 };
                let _ = self.channel.set_duty(map_duty(logical, self.max_duty));
                return;
            }
            acc = end;
        }
    }
}

fn sniffer_led_worker(health: Arc<AtomicU8>, mut led: SnifferLed) {
    task_wdt::register_current_task_for_watchdog();

    loop {
        task_wdt::reset_if_subscribed();
        let desired = LedHealth::from_wire(health.load(Ordering::Acquire));

        if desired == LedHealth::Healthy {
            if led.rendered != LedHealth::Healthy {
                led.leave_healthy();
                led.rendered = LedHealth::Healthy;
            }
            // Blocking fade halves run in this worker only; main loop stays free.
            if !led.breathe_once(&health) {
                led.leave_healthy();
            }
            continue;
        }

        if led.rendered == LedHealth::Healthy {
            led.leave_healthy();
        }
        led.rendered = desired;
        led.tick_fault(desired, stats::now_ms());
        task_wdt::sleep_feeding_wdt(Duration::from_millis(LED_TASK_POLL_MS));
    }
}

fn map_duty(logical: u32, max: u32) -> u32 {
    if LED_ACTIVE_HIGH {
        logical.min(max)
    } else {
        max.saturating_sub(logical.min(max))
    }
}
