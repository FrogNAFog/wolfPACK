//! GPS UART + NMEA reader.
//!
//! This module is intentionally small and self-contained:
//! - Reads bytes from a UART (typically UART1/UART2) connected to a GPS module.
//! - Parses NMEA RMC + GGA sentences (accepts talker variants: GP/GN/GL/GA/BD).
//! - Publishes the latest fix to shared state for other modules to consume.
//! - Forwards GPS UTC date+time to `time_sync::apply_gps_utc(...)` so the device
//!   wall clock (and therefore FAT file timestamps) become correct.
//! - **Position:** GGA-primary when `fix_quality >= 1`; else valid RMC (`A`). RMC↔GGA cross-check
//!   rejects disagreeing updates (>50 m within 2 s). Alt/sats come from recent GGA; HDOP is taken
//!   from GGA when fresh, else the last usable GGA HDOP within [`stats::GPS_HDOP_FALLBACK_MAX_AGE_MS`].
//! - **Clock:** RMC date+time via `time_sync::apply_gps_utc` on every RMC; wardriving gates use
//!   `stats::gps_nav_fresh` and `stats::gps_clock_stale` in the control loop.
//! - The reader thread never waits on `time_sync::wall_clock_trusted`; wardriving is gated in
//!   the control loop via `wall_clock_trusted` together with `gps_ok` (which requires measured HDOP).
//!
//! Notes:
//! - We do **not** depend on large NMEA crates to keep ESP-IDF builds stable.
//! - We intentionally do not print every sentence; callers should add rate-
//!   limited logging if desired.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_hal::uart::UartDriver;
use esp_idf_sys as sys;
use log::{info, warn};

use crate::geo;
use crate::time_sync;
use crate::wifi_scan::{self, MIN_FIX_QUALITY};
use wardriver_stats as stats;

/// Best-effort L76K setup: GPS+BeiDou+GLONASS, GGA+RMC NMEA only, 1 Hz.
///
/// Send before spawning the GNSS reader thread (and on hard recovery) so it does not race with parsing.
#[cfg(feature = "board-xiao")]
pub fn l76k_configure(uart: &mut UartDriver<'static>) -> Result<()> {
    l76k_send_pcas(uart, "PCAS04,7", "constellations GPS+BD+GLONASS")?;
    l76k_send_pcas(
        uart,
        "PCAS03,1,0,0,0,1,0,0,0,0,0,,,0,0",
        "NMEA GGA+RMC only",
    )?;
    l76k_send_pcas(uart, "PCAS02,1000", "1Hz")?;
    Ok(())
}

#[cfg(feature = "board-xiao")]
fn l76k_send_pcas(uart: &mut UartDriver<'static>, body: &str, label: &str) -> Result<()> {
    let cmd = pcas_with_checksum(body);
    match uart.write(cmd.as_bytes()) {
        Ok(_) => {
            info!("gps: L76K: sent {body} ({label}) (best-effort)");
        }
        Err(e) => {
            warn!("gps: L76K: failed to send {body} ({label}): {e}");
        }
    }
    Ok(())
}

fn pcas_with_checksum(body: &str) -> String {
    // body should not include '$' or '*' (example: "PCAS02,200").
    let csum = nmea_checksum(body.as_bytes());
    format!("${body}*{csum:02X}\r\n")
}

fn nmea_checksum(bytes: &[u8]) -> u8 {
    let mut x: u8 = 0;
    for &b in bytes {
        x ^= b;
    }
    x
}

#[derive(Clone, Debug)]
pub struct Fix {
    pub utc_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<f32>,
    pub hdop: Option<f32>,
    /// Satellites used for the fix (GGA field 7); `None` if the receiver omitted it.
    pub satellites: Option<u8>,
    pub fix_quality: u8, // 0 = invalid, 1 = GPS, 2 = DGPS, etc.
}

const GPS_HIST_PUSH_MIN_INTERVAL_MS: u64 = 500;

/// GNSS reader supervisor: shared UART, soft recovery in-thread, hard restart via [`restart_reader`].
pub struct GpsSupervisor {
    pub fix: Arc<Mutex<Option<Fix>>>,
    gps_hist: Option<Arc<RwLock<VecDeque<(i64, Fix)>>>>,
    stop: Arc<AtomicBool>,
    recovery: Arc<AtomicBool>,
    uart: Arc<Mutex<UartDriver<'static>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl GpsSupervisor {
    /// Spawn the reader thread; `uart` is shared so hard restart can re-spawn without new peripherals.
    pub fn start(
        uart: UartDriver<'static>,
        gps_hist: Option<Arc<RwLock<VecDeque<(i64, Fix)>>>>,
    ) -> Result<Arc<Self>> {
        let sup = Arc::new(Self {
            fix: Arc::new(Mutex::new(None)),
            gps_hist,
            stop: Arc::new(AtomicBool::new(false)),
            recovery: Arc::new(AtomicBool::new(false)),
            uart: Arc::new(Mutex::new(uart)),
            thread: Mutex::new(None),
        });
        sup.spawn_thread()?;
        Ok(sup)
    }

    pub fn request_soft_recovery(&self) {
        self.recovery.store(true, Ordering::Release);
    }

    /// Stop reader, flush/reconfigure UART, spawn a new reader thread.
    pub fn restart_reader(self: &Arc<Self>) -> Result<()> {
        stats::record_gps_recovery_hard_attempt();
        self.stop.store(true, Ordering::Release);
        if let Ok(mut g) = self.thread.lock() {
            if let Some(h) = g.take() {
                let _ = h.join();
            }
        }
        self.stop.store(false, Ordering::Release);
        if let Ok(mut g) = self.fix.lock() {
            *g = None;
        }
        stats::reset_gps_rate_observability();
        {
            let mut uart = self
                .uart
                .lock()
                .map_err(|_| anyhow!("gps: uart mutex poisoned"))?;
            uart_flush_rx(&mut *uart)?;
            #[cfg(feature = "board-xiao")]
            l76k_configure(&mut *uart)?;
        }
        self.spawn_thread()?;
        stats::record_gps_recovery_hard_ok();
        info!("gps: hard reader restart complete");
        Ok(())
    }

    fn spawn_thread(self: &Arc<Self>) -> Result<()> {
        let stop = Arc::clone(&self.stop);
        let recovery = Arc::clone(&self.recovery);
        let uart = Arc::clone(&self.uart);
        let state = Arc::clone(&self.fix);
        let gps_hist = self.gps_hist.clone();
        let h = thread::Builder::new()
            .name("gps".into())
            .stack_size(4096)
            .spawn(move || {
                if let Err(e) = gps_task(uart, state, gps_hist, stop, recovery) {
                    warn!("gps: task exited: {e:#}");
                }
            })?;
        if let Ok(mut g) = self.thread.lock() {
            *g = Some(h);
        }
        Ok(())
    }
}

/// Legacy entry: returns the shared fix handle only.
pub fn start(uart: UartDriver<'static>) -> Result<Arc<Mutex<Option<Fix>>>> {
    Ok(Arc::clone(&GpsSupervisor::start(uart, None)?.fix))
}

fn unix_now_ms() -> Option<u64> {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, core::ptr::null_mut());
    }
    if tv.tv_sec <= 0 {
        return None;
    }
    let ms = (tv.tv_sec as i64)
        .saturating_mul(1000)
        .saturating_add((tv.tv_usec as i64) / 1000);
    if ms <= 0 {
        return None;
    }
    Some(ms as u64)
}

fn maybe_record_gps_hist(
    gps_hist: &Option<Arc<RwLock<VecDeque<(i64, Fix)>>>>,
    last_hist_push_ms: &mut u64,
    now_ms: u64,
    fix: &Fix,
) {
    let Some(hist) = gps_hist else {
        return;
    };
    if !wifi_scan::is_fix_usable_for_location(fix) {
        return;
    }
    if *last_hist_push_ms != 0
        && now_ms.saturating_sub(*last_hist_push_ms) < GPS_HIST_PUSH_MIN_INTERVAL_MS
    {
        return;
    }
    let Some(sample_unix_ms) = unix_now_ms().map(|v| v as i64) else {
        return;
    };
    if hist
        .write()
        .ok()
        .map(|mut h| h.push_back((sample_unix_ms, fix.clone())))
        .is_some()
    {
        stats::gps_hist_sample_recorded();
        *last_hist_push_ms = now_ms;
    }
}

fn uart_flush_rx(uart: &mut UartDriver<'static>) -> Result<()> {
    let mut drain = [0u8; 64];
    for _ in 0..32 {
        match uart.read(&mut drain, 50) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

fn run_soft_recovery(uart: &mut UartDriver<'static>, line: &mut Vec<u8>) {
    stats::record_gps_recovery_soft_attempt();
    line.clear();
    let _ = uart_flush_rx(uart);
    stats::record_gps_recovery_soft_ok();
    info!("gps: soft recovery (RX flush)");
}

#[derive(Clone, Copy)]
struct GoodPos {
    lat: f64,
    lon: f64,
    publish_ms: u64,
}

fn gga_supplement_fresh(now_ms: u64, last_gga_parse_ms: u64) -> bool {
    last_gga_parse_ms != 0
        && now_ms.saturating_sub(last_gga_parse_ms) <= stats::GPS_NAV_FRESH_MAX_AGE_MS
}

fn hdop_value_usable(hdop: Option<f32>) -> bool {
    hdop.map(|h| h.is_finite() && h > 0.0 && h <= wifi_scan::MAX_HDOP)
        .unwrap_or(false)
}

/// HDOP for RMC merges: live GGA supplement, else last measured HDOP within the fallback window.
fn hdop_for_publish(
    now_ms: u64,
    last_gga_parse_ms: u64,
    last_hdop_valid_ms: u64,
    last_hdop: Option<f32>,
) -> Option<f32> {
    if !hdop_value_usable(last_hdop) {
        return None;
    }
    if gga_supplement_fresh(now_ms, last_gga_parse_ms) {
        return last_hdop;
    }
    if last_hdop_valid_ms != 0
        && now_ms.saturating_sub(last_hdop_valid_ms) <= stats::GPS_HDOP_FALLBACK_MAX_AGE_MS
    {
        return last_hdop;
    }
    None
}

fn cross_check_ok(lat: f64, lon: f64, now_ms: u64, other: Option<GoodPos>) -> bool {
    let Some(other) = other else {
        return true;
    };
    if now_ms.saturating_sub(other.publish_ms) > stats::GPS_CROSS_CHECK_MAX_DT_MS {
        return true;
    }
    let d_m = geo::haversine_m(lat, lon, other.lat, other.lon);
    if d_m > stats::GPS_CROSS_CHECK_MAX_M {
        stats::gps_cross_check_reject();
        false
    } else {
        true
    }
}

fn utc_from_gga_time(gga: &Gga, calendar: Option<(u16, u8, u8)>) -> Option<i64> {
    let (year, month, day) = calendar?;
    ymd_hms_to_unix_ms(
        year, month, day, gga.hour, gga.minute, gga.second, gga.millis,
    )
}

/// Mutable per-sentence state shared by the GGA / RMC handlers in [`gps_task`].
struct GpsSentenceState {
    last_hist_push_ms: u64,
    last_fix_quality: u8,
    last_hdop: Option<f32>,
    last_hdop_valid_ms: u64,
    last_alt_m: Option<f32>,
    last_satellites: Option<u8>,
    last_gga_parse_ms: u64,
    last_rmc_calendar: Option<(u16, u8, u8)>,
    last_good_rmc: Option<GoodPos>,
    last_good_gga: Option<GoodPos>,
    state: Arc<Mutex<Option<Fix>>>,
    gps_hist: Option<Arc<RwLock<VecDeque<(i64, Fix)>>>>,
}

/// Handle a GGA sentence: update the shared GGA fields and publish a fix when the
/// quality and RMC cross-check pass.
fn handle_gga(s: &mut GpsSentenceState, gga: &Gga) {
    stats::gps_nmea_parsed_gga();
    let now_ms = stats::now_ms();
    s.last_gga_parse_ms = now_ms;
    s.last_fix_quality = gga.fix_quality;
    s.last_hdop = gga.hdop;
    if hdop_value_usable(gga.hdop) {
        s.last_hdop_valid_ms = now_ms;
    }
    s.last_alt_m = gga.altitude_m;
    s.last_satellites = gga.satellites;
    stats::set_gps_satellites_in_view(gga.satellites);

    if gga.fix_quality >= MIN_FIX_QUALITY {
        if let Some(unix_ms) = utc_from_gga_time(gga, s.last_rmc_calendar) {
            if cross_check_ok(gga.lat, gga.lon, now_ms, s.last_good_rmc) {
                let fix = Fix {
                    utc_unix_ms: unix_ms,
                    lat: gga.lat,
                    lon: gga.lon,
                    altitude_m: gga.altitude_m.or(s.last_alt_m),
                    hdop: gga.hdop.or(s.last_hdop),
                    satellites: gga.satellites,
                    fix_quality: gga.fix_quality,
                };
                if let Ok(mut g) = s.state.lock() {
                    *g = Some(fix.clone());
                }
                stats::gps_nav_gga_valid_published(now_ms);
                maybe_record_gps_hist(&s.gps_hist, &mut s.last_hist_push_ms, now_ms, &fix);
                s.last_good_gga = Some(GoodPos {
                    lat: gga.lat,
                    lon: gga.lon,
                    publish_ms: now_ms,
                });
            }
        }
    } else if let Ok(mut g) = s.state.lock() {
        if let Some(fix) = g.as_mut() {
            fix.satellites = gga.satellites;
            fix.hdop = hdop_for_publish(
                now_ms,
                s.last_gga_parse_ms,
                s.last_hdop_valid_ms,
                gga.hdop.or(s.last_hdop),
            );
        }
    }
}

/// Handle an RMC sentence: update the calendar, apply UTC, and publish / refresh the
/// fix when the RMC is valid and the GGA cross-check passes. Returns `Ok(())` on the
/// invalid-RMC-date bail (the caller stops processing that sentence).
fn handle_rmc(s: &mut GpsSentenceState, rmc: &Rmc) -> Result<()> {
    stats::gps_nmea_parsed_rmc();
    stats::gps_set_last_rmc_nav_valid(rmc.valid);
    let now_ms = stats::now_ms();

    s.last_rmc_calendar = Some((rmc.year, rmc.month, rmc.day));

    time_sync::apply_gps_utc(
        rmc.year, rmc.month, rmc.day, rmc.hour, rmc.minute, rmc.second, rmc.millis,
    );

    if rmc.valid {
        let unix_ms = ymd_hms_to_unix_ms(
            rmc.year, rmc.month, rmc.day, rmc.hour, rmc.minute, rmc.second, rmc.millis,
        )
        .ok_or_else(|| anyhow!("gps: invalid RMC date"))?;

        let hdop = hdop_for_publish(
            now_ms,
            s.last_gga_parse_ms,
            s.last_hdop_valid_ms,
            s.last_hdop,
        );
        let alt = if gga_supplement_fresh(now_ms, s.last_gga_parse_ms) {
            s.last_alt_m
        } else {
            None
        };
        let sats = if gga_supplement_fresh(now_ms, s.last_gga_parse_ms) {
            s.last_satellites
        } else {
            None
        };

        if s.last_fix_quality >= MIN_FIX_QUALITY {
            if let Ok(mut g) = s.state.lock() {
                if let Some(fix) = g.as_mut() {
                    fix.utc_unix_ms = unix_ms;
                    fix.hdop = hdop;
                    fix.altitude_m = alt;
                    fix.satellites = sats;
                    stats::gps_nav_rmc_valid_published(now_ms);
                    maybe_record_gps_hist(&s.gps_hist, &mut s.last_hist_push_ms, now_ms, fix);
                    s.last_good_rmc = Some(GoodPos {
                        lat: rmc.lat,
                        lon: rmc.lon,
                        publish_ms: now_ms,
                    });
                }
            }
        } else if cross_check_ok(rmc.lat, rmc.lon, now_ms, s.last_good_gga) {
            let fix = Fix {
                utc_unix_ms: unix_ms,
                lat: rmc.lat,
                lon: rmc.lon,
                altitude_m: alt,
                hdop,
                satellites: sats,
                fix_quality: s.last_fix_quality,
            };
            if let Ok(mut g) = s.state.lock() {
                *g = Some(fix.clone());
            }
            stats::gps_nav_rmc_valid_published(now_ms);
            maybe_record_gps_hist(&s.gps_hist, &mut s.last_hist_push_ms, now_ms, &fix);
            s.last_good_rmc = Some(GoodPos {
                lat: rmc.lat,
                lon: rmc.lon,
                publish_ms: now_ms,
            });
        }
    }
    Ok(())
}

fn gps_task(
    uart: Arc<Mutex<UartDriver<'static>>>,
    state: Arc<Mutex<Option<Fix>>>,
    gps_hist: Option<Arc<RwLock<VecDeque<(i64, Fix)>>>>,
    stop: Arc<AtomicBool>,
    recovery: Arc<AtomicBool>,
) -> Result<()> {
    let mut gs = GpsSentenceState {
        last_hist_push_ms: 0,
        last_fix_quality: 0,
        last_hdop: None,
        last_hdop_valid_ms: 0,
        last_alt_m: None,
        last_satellites: None,
        last_gga_parse_ms: 0,
        last_rmc_calendar: None,
        last_good_rmc: None,
        last_good_gga: None,
        state,
        gps_hist,
    };

    // NMEA sentences are usually <= 82 chars, but allow extra room.
    let mut line: Vec<u8> = Vec::with_capacity(128);
    let mut buf = [0u8; 64];

    info!("gps: reader thread started");

    while !stop.load(Ordering::Acquire) {
        if recovery.swap(false, Ordering::AcqRel) {
            if let Ok(mut u) = uart.lock() {
                run_soft_recovery(&mut *u, &mut line);
            }
        }

        let n = {
            let u = uart
                .lock()
                .map_err(|_| anyhow!("gps: uart mutex poisoned"))?;
            match u.read(&mut buf, 200) {
                Ok(n) => n,
                Err(e) => {
                    warn!("gps: uart read error: {e}");
                    0
                }
            }
        };

        if n == 0 {
            thread::sleep(Duration::from_millis(10));
            continue;
        }

        stats::gps_uart_rx_batch(stats::now_ms(), n);

        for &b in &buf[..n] {
            if b == b'\r' {
                continue;
            }
            if b == b'\n' {
                if line.is_empty() {
                    continue;
                }
                if let Ok(s) = std::str::from_utf8(&line) {
                    let s = s.trim();
                    match classify_nmea_checksum(s) {
                        NmeaChecksum::Bad => stats::gps_nmea_checksum_bad(),
                        NmeaChecksum::Good => {
                            if let Some(msg) = parse_nmea_sentence(s) {
                                match msg {
                                    NmeaMsg::Gga(gga) => {
                                        handle_gga(&mut gs, &gga);
                                    }
                                    NmeaMsg::Rmc(rmc) => {
                                        if let Err(e) = handle_rmc(&mut gs, &rmc) {
                                            warn!("gps: {e:#}");
                                        }
                                    }
                                }
                            }
                        }
                        NmeaChecksum::NotNmeaish => {}
                    }
                }
                line.clear();
            } else if line.len() < 255 {
                line.push(b);
            } else {
                // If the line is malformed/too long, reset.
                line.clear();
            }
        }
    }
    info!("gps: reader thread stopping");
    Ok(())
}

// -------- NMEA parsing --------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NmeaChecksum {
    /// Sentence looks NMEA-ish and checksum is valid.
    Good,
    /// Sentence looks NMEA-ish but checksum is invalid.
    Bad,
    /// Not in a format we consider NMEA-ish for counting checksum failures.
    NotNmeaish,
}

fn classify_nmea_checksum(s: &str) -> NmeaChecksum {
    // We only count checksum failures for sentences that look like `$...*HH`.
    let Some(s) = s.strip_prefix('$') else {
        return NmeaChecksum::NotNmeaish;
    };
    let Some((payload, checksum_hex)) = s.split_once('*') else {
        return NmeaChecksum::NotNmeaish;
    };
    if payload.is_empty() || checksum_hex.len() < 2 {
        return NmeaChecksum::NotNmeaish;
    }
    let Ok(want) = u8::from_str_radix(&checksum_hex[..2], 16) else {
        return NmeaChecksum::NotNmeaish;
    };
    let mut got: u8 = 0;
    for b in payload.as_bytes() {
        got ^= *b;
    }
    if got == want {
        NmeaChecksum::Good
    } else {
        NmeaChecksum::Bad
    }
}

#[derive(Clone, Debug)]
enum NmeaMsg {
    Rmc(Rmc),
    Gga(Gga),
}

#[derive(Clone, Debug)]
struct Rmc {
    valid: bool,
    hour: u8,
    minute: u8,
    second: u8,
    millis: u16,
    day: u8,
    month: u8,
    year: u16,
    lat: f64,
    lon: f64,
}

#[derive(Clone, Debug)]
struct Gga {
    hour: u8,
    minute: u8,
    second: u8,
    millis: u16,
    lat: f64,
    lon: f64,
    fix_quality: u8,
    satellites: Option<u8>,
    hdop: Option<f32>,
    altitude_m: Option<f32>,
}

fn parse_nmea_sentence(s: &str) -> Option<NmeaMsg> {
    // Must start with '$', include '*', and pass checksum.
    let s = s.strip_prefix('$')?;
    let (payload, checksum_hex) = s.split_once('*')?;
    if checksum_hex.len() < 2 {
        return None;
    }
    let want = u8::from_str_radix(&checksum_hex[..2], 16).ok()?;
    let mut got: u8 = 0;
    for b in payload.as_bytes() {
        got ^= *b;
    }
    if got != want {
        return None;
    }

    let mut it = payload.split(',');
    let typ = it.next()?; // e.g. GPRMC, GNGGA
    if typ.len() < 3 {
        return None;
    }
    let msg = &typ[2..]; // drop talker id

    match msg {
        "RMC" => parse_rmc(it).map(NmeaMsg::Rmc),
        "GGA" => parse_gga(it).map(NmeaMsg::Gga),
        _ => None,
    }
}

fn parse_rmc<'a>(mut it: impl Iterator<Item = &'a str>) -> Option<Rmc> {
    // RMC fields: 1 time, 2 status, 3 lat, 4 N/S, 5 lon, 6 E/W, ... 9 date.
    let time = it.next()?; // hhmmss.sss
    let status = it.next()?; // A/V
    let lat = it.next()?;
    let ns = it.next()?;
    let lon = it.next()?;
    let ew = it.next()?;

    // Skip speed/course.
    let _speed = it.next();
    let _course = it.next();

    let date = it.next()?; // ddmmyy

    let (hour, minute, second, millis) = parse_hhmmss_sss(time)?;
    let (day, month, year) = parse_ddmmyy(date)?;
    let valid = status.as_bytes().first().copied() == Some(b'A');

    let lat = parse_lat_lon(lat, ns, true)?;
    let lon = parse_lat_lon(lon, ew, false)?;

    Some(Rmc {
        valid,
        hour,
        minute,
        second,
        millis,
        day,
        month,
        year,
        lat,
        lon,
    })
}

fn parse_gga<'a>(mut it: impl Iterator<Item = &'a str>) -> Option<Gga> {
    // GGA fields: 1 time, 2 lat, 3 N/S, 4 lon, 5 E/W, 6 fix quality, 7 sats, 8 hdop, 9 alt, 10 'M'
    let time = it.next()?;
    let lat_s = it.next()?;
    let ns = it.next()?;
    let lon_s = it.next()?;
    let ew = it.next()?;
    let fixq = it.next().unwrap_or("0");
    let sats = it.next();
    let hdop = it.next();
    let alt = it.next();

    let (hour, minute, second, millis) = parse_hhmmss_sss(time)?;
    let lat = parse_lat_lon(lat_s, ns, true)?;
    let lon = parse_lat_lon(lon_s, ew, false)?;

    let fix_quality = fixq.parse::<u8>().ok().unwrap_or(0);
    let satellites = sats.and_then(|v| {
        if v.is_empty() {
            None
        } else {
            v.parse::<u8>().ok()
        }
    });
    let hdop = hdop.and_then(|v| v.parse::<f32>().ok());
    let altitude_m = alt.and_then(|v| v.parse::<f32>().ok());

    Some(Gga {
        hour,
        minute,
        second,
        millis,
        lat,
        lon,
        fix_quality,
        satellites,
        hdop,
        altitude_m,
    })
}

fn parse_hhmmss_sss(s: &str) -> Option<(u8, u8, u8, u16)> {
    if s.len() < 6 {
        return None;
    }
    let hour = s.get(0..2)?.parse().ok()?;
    let minute = s.get(2..4)?.parse().ok()?;
    let second = s.get(4..6)?.parse().ok()?;
    let millis = if let Some((_, frac)) = s.split_once('.') {
        // Take up to 3 digits, pad right.
        let mut ms = 0u16;
        let mut mul = 100u16;
        for ch in frac.chars().take(3) {
            if let Some(d) = ch.to_digit(10) {
                ms += (d as u16) * mul;
                mul /= 10;
            } else {
                break;
            }
        }
        ms
    } else {
        0
    };
    Some((hour, minute, second, millis))
}

fn parse_ddmmyy(s: &str) -> Option<(u8, u8, u16)> {
    if s.len() != 6 {
        return None;
    }
    let day: u8 = s.get(0..2)?.parse().ok()?;
    let month: u8 = s.get(2..4)?.parse().ok()?;
    let yy: u8 = s.get(4..6)?.parse().ok()?;
    // NMEA RMC date is ddmmyy; assume 2000-2099 for yy 00..99.
    let year: u16 = 2000 + yy as u16;
    Some((day, month, year))
}

fn parse_lat_lon(value: &str, hemi: &str, is_lat: bool) -> Option<f64> {
    if value.is_empty() || hemi.is_empty() {
        return None;
    }
    let hemi = hemi.as_bytes().first().copied()?;
    let (deg_len, sign) = match (is_lat, hemi) {
        (true, b'N') => (2usize, 1.0),
        (true, b'S') => (2usize, -1.0),
        (false, b'E') => (3usize, 1.0),
        (false, b'W') => (3usize, -1.0),
        _ => return None,
    };
    if value.len() < deg_len + 2 {
        return None;
    }
    let deg: f64 = value.get(0..deg_len)?.parse().ok()?;
    let min: f64 = value.get(deg_len..)?.parse().ok()?;
    Some(sign * (deg + min / 60.0))
}

// -------- Time conversion helpers --------

fn ymd_hms_to_unix_ms(year: u16, month: u8, day: u8, h: u8, m: u8, s: u8, ms: u16) -> Option<i64> {
    if !(1970..=2099).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || h > 23
        || m > 59
        || s > 60
        || ms > 999
    {
        return None;
    }
    let days = days_from_civil(year as i32, month as u32, day as u32);
    Some(
        days * 86_400_000
            + (h as i64) * 3_600_000
            + (m as i64) * 60_000
            + (s as i64) * 1000
            + ms as i64,
    )
}

/// Days-from-civil algorithm (Howard Hinnant).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as i64) * 146_097 + doe as i64 - 719_468
}
