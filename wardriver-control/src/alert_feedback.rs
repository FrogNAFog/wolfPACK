//! Expansion-board buzzer + OLED alerts (`board-xiao` control builds).
//!
//! Events are queued to a worker thread so ESP-NOW / scan paths never block on LEDC tones.
//! **Dwell policy:** each message stays on the OLED for at least [`MIN_DWELL_MS`]; additional
//! events wait in the FIFO and are shown after the current dwell completes (no preemption).

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use log::{info, warn};

#[cfg(feature = "board-xiao")]
use crate::buzzer::Buzzer;
use wardriver_stats as stats;

pub use wardriver_stats::alert_kinds::{AlertKind, ALERT_SOUND_MASK_ALL};

/// Minimum time a message remains on the OLED before the next queued event is shown.
pub const MIN_DWELL_MS: u64 = 4000;

/// How long **BOOT** stays before post-boot GPS status (**NO GPS** / **NO FIX**).
const BOOT_DWELL_MS: u64 = 1500;

/// Extra time after **BOOT** for the L76K to start NMEA before UART-alive check.
const POST_BOOT_GPS_CHECK_MS: u64 = 500;

/// Rotate among active error lines (**NO SD** / **NO GPS** / **NO FIX**) on the OLED.
pub const STATUS_CYCLE_MS: u64 = 3000;

/// Cooldown for repeated detection / surveillance alerts (avoids buzzer lockout during bursts).
const DETECTION_COOLDOWN_MS: u64 = 30_000;

struct Tone {
    freq_hz: u32,
    duration_ms: u32,
    gap_ms: u32,
}

const fn tone(f: u16, d: u32, g: u32) -> Tone {
    Tone {
        freq_hz: f as u32,
        duration_ms: d,
        gap_ms: g,
    }
}

const BOOT: &[Tone] = &[tone(2700, 80, 0)];
const UPLOAD_START: &[Tone] = &[tone(1800, 60, 80), tone(1800, 60, 0)];
const UPLOAD_DONE: &[Tone] = &[tone(1200, 50, 60), tone(1800, 50, 60), tone(2400, 50, 0)];
const GPS_FIX: &[Tone] = &[tone(1500, 120, 40), tone(1500, 40, 40), tone(1500, 120, 0)];
const CAMERA: &[Tone] = &[tone(2200, 40, 50), tone(2200, 40, 50), tone(2200, 40, 0)];
const SURVEILLANCE: &[Tone] = &[tone(800, 80, 60), tone(800, 80, 60), tone(2400, 80, 0)];
// Ascending four-note chime — distinct from LPR-camera triple and Surveillance low/high.
const SSID_WATCH: &[Tone] = &[
    tone(1200, 50, 40),
    tone(1500, 50, 40),
    tone(1800, 50, 40),
    tone(2100, 50, 0),
];
const REMOTE_ID: &[Tone] = &[tone(988, 90, 50), tone(1319, 90, 50), tone(988, 90, 0)];
const SD_FAIL: &[Tone] = &[tone(500, 400, 0)];
const FLEET_UNBOUND: &[Tone] = &[tone(600, 120, 80), tone(600, 120, 80), tone(600, 120, 0)];
const NONE: &[Tone] = &[];

fn pattern_for(kind: AlertKind) -> &'static [Tone] {
    match kind {
        AlertKind::Boot => BOOT,
        AlertKind::UploadStart => UPLOAD_START,
        AlertKind::UploadDone => UPLOAD_DONE,
        AlertKind::GpsFix => GPS_FIX,
        AlertKind::Camera => CAMERA,
        AlertKind::Surveillance => SURVEILLANCE,
        AlertKind::SsidWatch => SSID_WATCH,
        AlertKind::RemoteId => REMOTE_ID,
        AlertKind::SdFail => SD_FAIL,
        AlertKind::FleetUnbound => FLEET_UNBOUND,
        AlertKind::GpsWaiting | AlertKind::GpsNoModule => NONE,
    }
}

/// Post-boot OLED status (resolved in alert worker after **BOOT** dwell).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostBootGpsOled {
    /// Show first active status line; main loop rotates all active faults every [`STATUS_CYCLE_MS`].
    Deferred { gps_ok: bool },
}

/// One OLED status rotation line (title + optional subtitle).
pub struct OledStatusLine {
    pub title: &'static str,
    pub subtitle: Option<String>,
}

fn gps_oled_subtitle(fix: Option<&crate::gps::Fix>) -> Option<String> {
    let mut parts = Vec::new();
    let sats = fix
        .and_then(|f| f.satellites)
        .or_else(stats::gps_satellites_in_view);
    if let Some(n) = sats {
        parts.push(format!("{n} sats"));
    }
    if let Some(h) = fix.and_then(|f| f.hdop) {
        parts.push(wardriver_stats::json_fixed::hdop_subtitle_token(h));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" - "))
    }
}

/// Active OLED lines to rotate: faults first, then GPS OK with satellite count when logging.
pub fn oled_status_lines(gps_ok: bool, fix: Option<&crate::gps::Fix>) -> Vec<OledStatusLine> {
    let mut msgs = Vec::new();
    if !stats::sd_mounted() {
        msgs.push(OledStatusLine {
            title: "NO SD",
            subtitle: None,
        });
    }
    if !gps_ok {
        if stats::gps_uart_alive() {
            let sub = gps_oled_subtitle(fix);
            msgs.push(OledStatusLine {
                title: "NO FIX",
                subtitle: sub,
            });
        } else {
            msgs.push(OledStatusLine {
                title: "NO GPS",
                subtitle: None,
            });
        }
    } else {
        msgs.push(OledStatusLine {
            title: "GPS OK",
            subtitle: gps_oled_subtitle(fix),
        });
    }
    msgs
}

#[cfg(feature = "board-xiao")]
fn play_pattern(buzzer: &mut Buzzer<'_>, kind: AlertKind) {
    for t in pattern_for(kind) {
        buzzer.beep(t.freq_hz, t.duration_ms);
        if t.gap_ms > 0 {
            thread::sleep(Duration::from_millis(u64::from(t.gap_ms)));
        }
    }
}

struct AlertEvent {
    kind: AlertKind,
    /// When set, used instead of [`AlertKind::title`] on the OLED.
    title_override: Option<String>,
    subtitle: Option<String>,
    post_boot_gps: Option<PostBootGpsOled>,
}

fn oled_dwell_ms(kind: AlertKind) -> u64 {
    match kind {
        AlertKind::Boot => BOOT_DWELL_MS,
        k if k.is_persistent_status() => 0,
        _ => MIN_DWELL_MS,
    }
}

#[cfg(feature = "board-xiao")]
pub struct AlertHub {
    tx: SyncSender<AlertEvent>,
    display_slot: Arc<Mutex<Option<crate::display_ssd1306::Display>>>,
}

#[cfg(feature = "board-xiao")]
impl AlertHub {
    pub fn spawn(buzzer: Buzzer<'static>) -> Result<Arc<Self>> {
        let (tx, rx) = sync_channel::<AlertEvent>(8);
        let display_slot = Arc::new(Mutex::new(None::<crate::display_ssd1306::Display>));
        let display_worker = Arc::clone(&display_slot);
        let sound_mask = stats::alert_sound_mask();
        thread::Builder::new()
            .name("alert".into())
            .stack_size(16_384)
            .spawn(move || {
                let mut buzzer = buzzer;
                info!("alert: worker started display=none sound_mask=0x{sound_mask:02x}");
                let mut last_detection_ms: u64 = 0;
                let mut last_axon_ms: u64 = 0;
                let mut last_ssid_watch_ms: u64 = 0;
                let mut last_remote_id_ms: u64 = 0;
                let mut logged_display = false;
                while let Ok(ev) = rx.recv() {
                    if !logged_display {
                        if display_worker.lock().ok().is_some_and(|g| g.is_some()) {
                            info!("alert: worker display=present");
                            logged_display = true;
                        }
                    }

                    let now = stats::now_ms();
                    match ev.kind {
                        AlertKind::Camera => {
                            if now.saturating_sub(last_detection_ms) < DETECTION_COOLDOWN_MS {
                                continue;
                            }
                            last_detection_ms = now;
                        }
                        AlertKind::Surveillance => {
                            if now.saturating_sub(last_axon_ms) < DETECTION_COOLDOWN_MS {
                                continue;
                            }
                            last_axon_ms = now;
                        }
                        AlertKind::SsidWatch => {
                            if now.saturating_sub(last_ssid_watch_ms) < DETECTION_COOLDOWN_MS {
                                continue;
                            }
                            last_ssid_watch_ms = now;
                        }
                        AlertKind::RemoteId => {
                            if now.saturating_sub(last_remote_id_ms) < DETECTION_COOLDOWN_MS {
                                continue;
                            }
                            last_remote_id_ms = now;
                        }
                        _ => {}
                    }

                    let play_sound = ev.kind.plays_sound()
                        && (!ev.kind.is_user_configurable() || stats::alert_sound_enabled(ev.kind));
                    if play_sound {
                        play_pattern(&mut buzzer, ev.kind);
                        if ev.kind == AlertKind::Boot {
                            info!("alert: played Boot pattern");
                        }
                    }

                    // Upload progress/completion OLED is driven by boot_setup, not the queue.
                    if ev.kind == AlertKind::UploadStart || ev.kind == AlertKind::UploadDone {
                        continue;
                    }

                    if ev.kind == AlertKind::SdFail {
                        continue;
                    }

                    // Buzzer still fires; upload handler owns the OLED until hold expires.
                    if stats::upload_in_progress() || stats::upload_oled_hold_active() {
                        continue;
                    }

                    let dwell_ms = oled_dwell_ms(ev.kind);
                    let title = ev.title_override.as_deref().unwrap_or(ev.kind.title());
                    {
                        if let Ok(mut g) = display_worker.lock() {
                            if let Some(ref mut disp) = *g {
                                let sub = ev.subtitle.as_deref();
                                if let Err(e) = disp.show_message(title, sub) {
                                    warn!("alert: OLED show {:?}: {e:#}", ev.kind);
                                }
                            }
                        }
                    }
                    if dwell_ms > 0 {
                        thread::sleep(Duration::from_millis(dwell_ms));
                    }

                    if ev.kind == AlertKind::Boot {
                        if let Some(PostBootGpsOled::Deferred { gps_ok }) = ev.post_boot_gps {
                            thread::sleep(Duration::from_millis(POST_BOOT_GPS_CHECK_MS));
                            let msgs = oled_status_lines(gps_ok, None);
                            if let Some(line) = msgs.first() {
                                if let Ok(mut g) = display_worker.lock() {
                                    if let Some(ref mut disp) = *g {
                                        if let Err(e) =
                                            disp.show_message(line.title, line.subtitle.as_deref())
                                        {
                                            warn!("alert: OLED {}: {e:#}", line.title);
                                        } else {
                                            info!(
                                                "alert: OLED -> {} ({} status line(s))",
                                                line.title,
                                                msgs.len()
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            })?;
        Ok(Arc::new(Self { tx, display_slot }))
    }

    pub fn attach_display(&self, display: crate::display_ssd1306::Display) {
        if let Ok(mut g) = self.display_slot.lock() {
            *g = Some(display);
            info!("alert: display attached to hub");
        }
    }

    /// Show a status line immediately (used for SD/GPS error rotation; bypasses the alert queue).
    pub fn show_oled_status(&self, title: &str, subtitle: Option<&str>) {
        if let Ok(mut g) = self.display_slot.lock() {
            if let Some(ref mut disp) = *g {
                if let Err(e) = disp.show_message(title, subtitle) {
                    warn!("alert: OLED status {title}: {e:#}");
                }
            }
        }
    }

    pub fn trigger(&self, kind: AlertKind, subtitle: Option<&str>) {
        self.send(AlertEvent {
            kind,
            title_override: None,
            subtitle: subtitle.map(str::to_string),
            post_boot_gps: None,
        });
    }

    /// Like [`Self::trigger`] but overrides the OLED title (e.g. `AXON` / `I-PRO`).
    pub fn trigger_with_title(&self, kind: AlertKind, title: &str, subtitle: Option<&str>) {
        self.send(AlertEvent {
            kind,
            title_override: Some(title.to_string()),
            subtitle: subtitle.map(str::to_string),
            post_boot_gps: None,
        });
    }

    /// Buzzer-only upload completion; OLED already updated by upload handler.
    pub fn trigger_upload_done(&self, title: &str, subtitle: &str) {
        self.send(AlertEvent {
            kind: AlertKind::UploadDone,
            title_override: Some(title.to_string()),
            subtitle: Some(subtitle.to_string()),
            post_boot_gps: None,
        });
    }

    /// Boot beep + **BOOT** briefly, then first status fault line (if any).
    pub fn trigger_boot(&self, post_boot_gps: Option<PostBootGpsOled>) {
        self.send(AlertEvent {
            kind: AlertKind::Boot,
            title_override: None,
            subtitle: None,
            post_boot_gps,
        });
    }

    fn send(&self, ev: AlertEvent) {
        let kind = ev.kind;
        if self.tx.try_send(ev).is_err() {
            warn!("alert: queue full, dropped {:?}", kind);
        }
    }
}

#[cfg(not(feature = "board-xiao"))]
pub struct AlertHub;

#[cfg(not(feature = "board-xiao"))]
impl AlertHub {
    pub fn spawn() -> Result<Arc<Self>> {
        Ok(Arc::new(Self))
    }

    pub fn attach_display(&self, _display: crate::display_ssd1306::Display) {}

    pub fn show_oled_status(&self, _title: &str, _subtitle: Option<&str>) {}

    pub fn trigger(&self, _kind: AlertKind, _subtitle: Option<&str>) {}

    pub fn trigger_with_title(&self, _kind: AlertKind, _title: &str, _subtitle: Option<&str>) {}

    pub fn trigger_boot(&self, _post_boot_gps: Option<PostBootGpsOled>) {}

    pub fn trigger_upload_done(&self, _title: &str, _subtitle: &str) {}
}
