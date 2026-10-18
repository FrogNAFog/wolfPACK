//! Shield passive buzzer on Seeed expansion base (`board-xiao`, **D3 / GPIO7**).
//!
//! Seeed maps the expansion “A3” net to **D3 (GPIO7)** on XIAO ESP32-C5, not GPIO4 (MTCK).
//! Passive buzzer needs PWM (LEDC), matching Seeed’s `analogWrite` / `tone()` examples.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use esp_idf_hal::gpio::OutputPin;
use esp_idf_hal::ledc::{
    config::TimerConfig, LedcChannel, LedcDriver, LedcTimer, LedcTimerDriver, LowSpeed, SpeedMode,
};
use esp_idf_hal::sys::esp;
use esp_idf_hal::units::Hertz;
use esp_idf_sys::ledc_set_freq;
use log::info;

pub struct Buzzer<'d> {
    channel: LedcDriver<'d>,
}

impl<'d> Buzzer<'d> {
    pub fn new<C, T>(channel: C, timer: T, pin: impl OutputPin + 'd) -> Result<Self>
    where
        C: LedcChannel<SpeedMode = LowSpeed> + 'd,
        T: LedcTimer<SpeedMode = LowSpeed> + 'd,
    {
        let timer_cfg = TimerConfig::new().frequency(Hertz(2_000));
        let timer = LedcTimerDriver::new(timer, &timer_cfg).context("buzzer LEDC timer")?;
        let mut channel =
            LedcDriver::new(channel, timer, pin).context("buzzer LEDC channel on GPIO7")?;
        channel.set_duty(0).context("buzzer duty off")?;
        info!("alert: buzzer on GPIO7 (D3, expansion A3 net) via LEDC");
        Ok(Self { channel })
    }

    fn set_frequency(&self, freq: Hertz) {
        let _ = esp!(unsafe { ledc_set_freq(LowSpeed::SPEED_MODE, self.channel.timer(), freq.0) });
    }

    /// Blocking tone at `freq_hz` for `duration_ms`, then silence.
    pub fn beep(&mut self, freq_hz: u32, duration_ms: u32) {
        if freq_hz == 0 || duration_ms == 0 {
            return;
        }
        let freq = Hertz(freq_hz.clamp(200, 10_000));
        self.set_frequency(freq);
        let max = self.channel.get_max_duty();
        let pct = wardriver_stats::alert_buzzer_volume_percent() as u32;
        let duty = max.saturating_mul(pct.min(100)) / 100;
        if self.channel.set_duty(duty).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(u64::from(duration_ms)));
        let _ = self.channel.set_duty(0);
    }
}
