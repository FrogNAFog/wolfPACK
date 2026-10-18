//! GPS observability (UART + NMEA + freshness gates).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::counters::now_ms;

type GpsRecoveryLogger = fn(&str, &str);

static GPS_RECOVERY_LOGGER: OnceLock<GpsRecoveryLogger> = OnceLock::new();

/// Control firmware registers SD diag logging for GPS recovery events.
#[cfg(feature = "control")]
pub fn install_gps_recovery_logger(f: GpsRecoveryLogger) {
    let _ = GPS_RECOVERY_LOGGER.set(f);
}

pub static GPS_TIME_VALID: AtomicBool = AtomicBool::new(false);

// -------- GPS observability (UART + NMEA) --------
//
// These are monotonic-time observability counters/timestamps for diagnosing GPS issues.
// Keep updates cheap; GPS task should batch-update per UART read or per completed line.

#[derive(Clone, Copy, Default)]
struct GpsUartRx {
    last_rx_ms: u64,
    rx_bytes: u64,
}

static GPS_UART_RX: Mutex<GpsUartRx> = Mutex::new(GpsUartRx {
    last_rx_ms: 0,
    rx_bytes: 0,
});

static GPS_NMEA_PARSED: Mutex<u64> = Mutex::new(0);
static GPS_NMEA_CHECKSUM_BAD: Mutex<u64> = Mutex::new(0);
static GPS_NMEA_RMC_COUNT: Mutex<u64> = Mutex::new(0);
static GPS_NMEA_GGA_COUNT: Mutex<u64> = Mutex::new(0);

/// 255 = never seen an RMC; 1 = last RMC status A; 0 = last RMC status V.
pub static GPS_LAST_RMC_NAV_VALID: AtomicU8 = AtomicU8::new(255);
/// 255 = unknown; otherwise last GGA satellites-in-use count (field 7).
static GPS_LAST_SATELLITES: AtomicU8 = AtomicU8::new(255);

/// Milliseconds between the last two RMC sentences parsed (200 ≈ 5 Hz, 1000 ≈ 1 Hz).
static GPS_RMC_INTERVAL_MS: AtomicU32 = AtomicU32::new(0);
/// Smoothed RMC rate over the last rate-tick window: 50 = 5.0 Hz (updated ~1 Hz for diagnostics).
static GPS_RMC_RATE_HZ_X10: AtomicU32 = AtomicU32::new(0);
/// WiGLE `gps_hist` snapshots that passed the location gate (control-side, not raw NMEA).
static GPS_HIST_SAMPLES: Mutex<u64> = Mutex::new(0);
/// Smoothed `gps_hist` sample rate (×10 Hz), same tick as [`GPS_RMC_RATE_HZ_X10`].
static GPS_HIST_RATE_HZ_X10: AtomicU32 = AtomicU32::new(0);
#[derive(Clone, Copy, Default)]
struct GpsRateTickState {
    last_ms: u64,
    last_rmc: u64,
    last_hist: u64,
}

static GPS_RATE_TICK: Mutex<GpsRateTickState> = Mutex::new(GpsRateTickState {
    last_ms: 0,
    last_rmc: 0,
    last_hist: 0,
});
static GPS_RMC_LAST_PARSE_MS: Mutex<u64> = Mutex::new(0);
static GPS_GGA_LAST_PARSE_MS: Mutex<u64> = Mutex::new(0);
#[derive(Clone, Copy, Default)]
struct GpsNavPublishMs {
    /// Last monotonic ms when a valid RMC (`A`) position was published to shared `Fix`.
    rmc: u64,
    /// Last monotonic ms when a valid GGA (`fix_quality >= 1`) position was published.
    gga: u64,
}

static GPS_NAV_PUBLISH_MS: Mutex<GpsNavPublishMs> = Mutex::new(GpsNavPublishMs { rmc: 0, gga: 0 });
static GPS_CROSS_CHECK_REJECTS: Mutex<u64> = Mutex::new(0);

/// Max age of UART RX or per-sentence parse for diagnostics ([`gps_rmc_fresh`], [`gps_gga_fresh`]).
pub const GPS_DATA_FRESH_MAX_AGE_MS: u64 = 2500;
/// Wardriving nav gate: recent valid position publish from RMC or GGA (~5× 1 Hz slack).
pub const GPS_NAV_FRESH_MAX_AGE_MS: u64 = 5000;
/// No RMC parse for this long → clock stale, gate `gps_ok`, trigger recovery even if GGA runs.
pub const GPS_CLOCK_STALE_MS: u64 = 30000;
/// RMC-primary updates may reuse the last GGA HDOP until this age.
pub const GPS_HDOP_FALLBACK_MAX_AGE_MS: u64 = 5000;
/// RMC↔GGA disagreement veto (haversine metres).
pub const GPS_CROSS_CHECK_MAX_M: f64 = 50.0;
/// Only cross-check when the other sentence's valid publish is within this age.
pub const GPS_CROSS_CHECK_MAX_DT_MS: u64 = 2000;

static GPS_STALE_EVENTS: Mutex<u64> = Mutex::new(0);
static GPS_RECOVERY_SOFT_ATTEMPTS: Mutex<u64> = Mutex::new(0);
static GPS_RECOVERY_SOFT_OK: Mutex<u64> = Mutex::new(0);
static GPS_RECOVERY_HARD_ATTEMPTS: Mutex<u64> = Mutex::new(0);
static GPS_RECOVERY_HARD_OK: Mutex<u64> = Mutex::new(0);
static GPS_HIST_DRAIN_OK: AtomicBool = AtomicBool::new(false);
static GPS_DATA_FRESH_PREV: AtomicBool = AtomicBool::new(false);
static GPS_RMC_FRESH_PREV: AtomicBool = AtomicBool::new(false);
static GPS_NAV_FRESH_PREV: AtomicBool = AtomicBool::new(false);

static WIFI_ROWS_DROPPED_GPS_STALE: Mutex<u64> = Mutex::new(0);
static BLE_ROWS_DROPPED_GPS_STALE: Mutex<u64> = Mutex::new(0);
pub fn set_gps_time_valid(v: bool) {
    GPS_TIME_VALID.store(v, Ordering::Release);
}

pub fn set_gps_hist_drain_ok(v: bool) {
    GPS_HIST_DRAIN_OK.store(v, Ordering::Release);
}

pub fn gps_hist_drain_ok() -> bool {
    GPS_HIST_DRAIN_OK.load(Ordering::Acquire)
}

pub fn gps_uart_rx_batch(now_ms: u64, n_bytes: usize) {
    if n_bytes == 0 {
        return;
    }
    if let Ok(mut g) = GPS_UART_RX.lock() {
        g.last_rx_ms = now_ms;
        g.rx_bytes = g.rx_bytes.saturating_add(n_bytes as u64);
    }
}

pub fn gps_nmea_parsed_rmc() {
    let now = now_ms();
    if let Ok(mut g) = GPS_RMC_LAST_PARSE_MS.lock() {
        let prev = *g;
        *g = now;
        if prev != 0 && now > prev {
            GPS_RMC_INTERVAL_MS.store(
                now.saturating_sub(prev).min(u32::MAX as u64) as u32,
                Ordering::Relaxed,
            );
        }
    }
    if let Ok(mut g) = GPS_NMEA_PARSED.lock() {
        *g = g.saturating_add(1);
    }
    if let Ok(mut g) = GPS_NMEA_RMC_COUNT.lock() {
        *g = g.saturating_add(1);
    }
}

/// Record one usable fix pushed into control `gps_hist` (2 Hz cap in the main loop).
pub fn gps_hist_sample_recorded() {
    if let Ok(mut g) = GPS_HIST_SAMPLES.lock() {
        *g = g.saturating_add(1);
    }
}

/// Recompute smoothed GNSS rates about once per second (call from the control main loop).
pub fn gps_rate_tick(now_ms: u64) {
    let Ok(mut tick) = GPS_RATE_TICK.lock() else {
        return;
    };
    if tick.last_ms != 0 && now_ms.saturating_sub(tick.last_ms) < 1000 {
        return;
    }
    let elapsed_ms = if tick.last_ms == 0 {
        1000
    } else {
        now_ms.saturating_sub(tick.last_ms).max(1)
    };
    tick.last_ms = now_ms;

    let rmc_now = gps_nmea_rmc_count();
    let hist_now = GPS_HIST_SAMPLES.lock().map(|g| *g).unwrap_or(0);
    let rmc_prev = tick.last_rmc;
    let hist_prev = tick.last_hist;

    let rmc_delta = rmc_now.saturating_sub(rmc_prev);
    let hist_delta = hist_now.saturating_sub(hist_prev);
    let elapsed_s = elapsed_ms as f64 / 1000.0;
    let rmc_hz_x10 = ((rmc_delta as f64 * 10.0) / elapsed_s).round() as u32;
    let hist_hz_x10 = ((hist_delta as f64 * 10.0) / elapsed_s).round() as u32;

    GPS_RMC_RATE_HZ_X10.store(rmc_hz_x10.min(255), Ordering::Relaxed);
    GPS_HIST_RATE_HZ_X10.store(hist_hz_x10.min(255), Ordering::Relaxed);

    tick.last_rmc = rmc_now;
    tick.last_hist = hist_now;
}

pub fn gps_rmc_interval_ms() -> u32 {
    GPS_RMC_INTERVAL_MS.load(Ordering::Relaxed)
}

/// RMC sentences per second (×10 internal); divide by 10 for Hz.
pub fn gps_rmc_rate_hz_x10() -> u32 {
    GPS_RMC_RATE_HZ_X10.load(Ordering::Relaxed)
}

/// Control `gps_hist` samples per second (×10 internal).
pub fn gps_hist_rate_hz_x10() -> u32 {
    GPS_HIST_RATE_HZ_X10.load(Ordering::Relaxed)
}

pub fn gps_nmea_parsed_gga() {
    let now = now_ms();
    if let Ok(mut g) = GPS_GGA_LAST_PARSE_MS.lock() {
        *g = now;
    }
    if let Ok(mut g) = GPS_NMEA_PARSED.lock() {
        *g = g.saturating_add(1);
    }
    if let Ok(mut g) = GPS_NMEA_GGA_COUNT.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn gps_nav_rmc_valid_published(now_ms: u64) {
    if let Ok(mut g) = GPS_NAV_PUBLISH_MS.lock() {
        g.rmc = now_ms;
    }
}

pub fn gps_nav_gga_valid_published(now_ms: u64) {
    if let Ok(mut g) = GPS_NAV_PUBLISH_MS.lock() {
        g.gga = now_ms;
    }
}

pub fn gps_cross_check_reject() {
    if let Ok(mut g) = GPS_CROSS_CHECK_REJECTS.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn gps_cross_check_rejects() -> u64 {
    GPS_CROSS_CHECK_REJECTS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_gga_last_parse_ms() -> u64 {
    GPS_GGA_LAST_PARSE_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_gga_last_parse_age_ms(now_ms: u64) -> u64 {
    let last = gps_gga_last_parse_ms();
    if last == 0 {
        u64::MAX
    } else {
        now_ms.saturating_sub(last)
    }
}

fn gps_nav_publish_age_ms(last: u64, now_ms: u64) -> u64 {
    if last == 0 {
        u64::MAX
    } else {
        now_ms.saturating_sub(last)
    }
}

/// True when a GGA sentence was parsed recently (diagnostics).
pub fn gps_gga_fresh(now_ms: u64) -> bool {
    let gga_age = gps_gga_last_parse_age_ms(now_ms);
    gga_age != u64::MAX && gga_age <= GPS_DATA_FRESH_MAX_AGE_MS
}

/// True when a valid position was published recently from RMC or GGA (wardriving gate).
pub fn gps_nav_fresh(now_ms: u64) -> bool {
    GPS_NAV_PUBLISH_MS
        .lock()
        .map(|g| {
            let rmc_pub = gps_nav_publish_age_ms(g.rmc, now_ms);
            let gga_pub = gps_nav_publish_age_ms(g.gga, now_ms);
            (rmc_pub != u64::MAX && rmc_pub <= GPS_NAV_FRESH_MAX_AGE_MS)
                || (gga_pub != u64::MAX && gga_pub <= GPS_NAV_FRESH_MAX_AGE_MS)
        })
        .unwrap_or(false)
}

/// True when no RMC parse for [`GPS_CLOCK_STALE_MS`] (blocks logging; triggers recovery).
pub fn gps_clock_stale(now_ms: u64) -> bool {
    let rmc_age = gps_rmc_last_parse_age_ms(now_ms);
    rmc_age != u64::MAX && rmc_age > GPS_CLOCK_STALE_MS
}

pub fn gps_nmea_checksum_bad() {
    if let Ok(mut g) = GPS_NMEA_CHECKSUM_BAD.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn gps_set_last_rmc_nav_valid(valid: bool) {
    GPS_LAST_RMC_NAV_VALID.store(if valid { 1 } else { 0 }, Ordering::Release);
}

/// Update from the latest GGA sentence (field 7: satellites used for fix).
pub fn set_gps_satellites_in_view(count: Option<u8>) {
    GPS_LAST_SATELLITES.store(count.unwrap_or(255).min(99), Ordering::Release);
}

/// Last GGA satellite count, if the receiver reported it.
pub fn gps_satellites_in_view() -> Option<u8> {
    let v = GPS_LAST_SATELLITES.load(Ordering::Acquire);
    if v == 255 {
        None
    } else {
        Some(v)
    }
}

pub fn gps_uart_last_rx_ms() -> u64 {
    GPS_UART_RX.lock().map(|g| g.last_rx_ms).unwrap_or(0)
}

pub fn gps_rmc_last_parse_ms() -> u64 {
    GPS_RMC_LAST_PARSE_MS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_uart_rx_age_ms(now_ms: u64) -> u64 {
    let last = gps_uart_last_rx_ms();
    if last == 0 {
        u64::MAX
    } else {
        now_ms.saturating_sub(last)
    }
}

pub fn gps_rmc_last_parse_age_ms(now_ms: u64) -> u64 {
    let last = gps_rmc_last_parse_ms();
    if last == 0 {
        u64::MAX
    } else {
        now_ms.saturating_sub(last)
    }
}

/// True when UART RX was recent (see [`GPS_DATA_FRESH_MAX_AGE_MS`]).
pub fn gps_uart_fresh(now_ms: u64) -> bool {
    let uart_age = gps_uart_rx_age_ms(now_ms);
    uart_age != u64::MAX && uart_age <= GPS_DATA_FRESH_MAX_AGE_MS
}

/// True when an RMC sentence was parsed recently (diagnostics only).
pub fn gps_rmc_fresh(now_ms: u64) -> bool {
    let rmc_age = gps_rmc_last_parse_age_ms(now_ms);
    rmc_age != u64::MAX && rmc_age <= GPS_DATA_FRESH_MAX_AGE_MS
}

/// True when UART or any GNSS sentence activity was recent (diagnostics only).
pub fn gps_data_fresh(now_ms: u64) -> bool {
    gps_uart_fresh(now_ms) || gps_rmc_fresh(now_ms) || gps_gga_fresh(now_ms)
}

fn log_gps_recovery_to_sd(kind: &str, detail: &str) {
    if let Some(f) = GPS_RECOVERY_LOGGER.get() {
        f(kind, detail);
    }
}

/// Call from the control loop ~5 s; logs nav fresh→stale (even when UART is still active).
pub fn gps_freshness_tick(now_ms: u64) {
    let nav_fresh = gps_nav_fresh(now_ms);
    let nav_prev = GPS_NAV_FRESH_PREV.swap(nav_fresh, Ordering::AcqRel);
    if nav_prev && !nav_fresh {
        if let Ok(mut g) = GPS_STALE_EVENTS.lock() {
            *g = g.saturating_add(1);
        }
        log::warn!(
            "gps: nav stale (uart_idle_ms={} rmc_parse_age_ms={} gga_parse_age_ms={} clock_stale={})",
            gps_uart_rx_age_ms(now_ms).min(u32::MAX as u64),
            gps_rmc_last_parse_age_ms(now_ms).min(u32::MAX as u64),
            gps_gga_last_parse_age_ms(now_ms).min(u32::MAX as u64),
            gps_clock_stale(now_ms) as u8
        );
    }
    let _ = GPS_RMC_FRESH_PREV.swap(gps_rmc_fresh(now_ms), Ordering::AcqRel);
    let _ = GPS_DATA_FRESH_PREV.swap(gps_data_fresh(now_ms), Ordering::AcqRel);
}

pub fn gps_stale_events() -> u64 {
    GPS_STALE_EVENTS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_recovery_soft_attempts() -> u64 {
    GPS_RECOVERY_SOFT_ATTEMPTS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_recovery_soft_ok() -> u64 {
    GPS_RECOVERY_SOFT_OK.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_recovery_hard_attempts() -> u64 {
    GPS_RECOVERY_HARD_ATTEMPTS.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_recovery_hard_ok() -> u64 {
    GPS_RECOVERY_HARD_OK.lock().map(|g| *g).unwrap_or(0)
}

pub fn record_gps_recovery_soft_attempt() {
    if let Ok(mut g) = GPS_RECOVERY_SOFT_ATTEMPTS.lock() {
        *g = g.saturating_add(1);
    }
    log_gps_recovery_to_sd("soft", "uart_flush=1");
}

pub fn record_gps_recovery_soft_ok() {
    if let Ok(mut g) = GPS_RECOVERY_SOFT_OK.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn record_gps_recovery_hard_attempt() {
    if let Ok(mut g) = GPS_RECOVERY_HARD_ATTEMPTS.lock() {
        *g = g.saturating_add(1);
    }
    log_gps_recovery_to_sd("hard", "reader_restart l76k_configure=1");
}

pub fn record_gps_recovery_hard_ok() {
    if let Ok(mut g) = GPS_RECOVERY_HARD_OK.lock() {
        *g = g.saturating_add(1);
    }
}

pub fn reset_gps_rate_observability() {
    GPS_RMC_LAST_PARSE_MS.lock().ok().map(|mut g| *g = 0);
    GPS_GGA_LAST_PARSE_MS.lock().ok().map(|mut g| *g = 0);
    GPS_NAV_PUBLISH_MS
        .lock()
        .ok()
        .map(|mut g| *g = GpsNavPublishMs::default());
    GPS_UART_RX.lock().ok().map(|mut g| {
        g.last_rx_ms = 0;
        g.rx_bytes = 0;
    });
    GPS_RMC_RATE_HZ_X10.store(0, Ordering::Relaxed);
    GPS_HIST_RATE_HZ_X10.store(0, Ordering::Relaxed);
    GPS_RMC_INTERVAL_MS.store(0, Ordering::Relaxed);
    GPS_LAST_SATELLITES.store(255, Ordering::Release);
}

pub fn wifi_rows_dropped_gps_stale() -> u64 {
    WIFI_ROWS_DROPPED_GPS_STALE.lock().map(|g| *g).unwrap_or(0)
}

pub fn ble_rows_dropped_gps_stale() -> u64 {
    BLE_ROWS_DROPPED_GPS_STALE.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_uart_rx_bytes() -> u64 {
    GPS_UART_RX.lock().map(|g| g.rx_bytes).unwrap_or(0)
}

/// True when the GNSS UART has delivered bytes or parsed NMEA (module likely connected).
pub fn gps_uart_alive() -> bool {
    gps_uart_rx_bytes() > 0 || gps_nmea_parsed() > 0
}

pub fn gps_nmea_parsed() -> u64 {
    GPS_NMEA_PARSED.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_nmea_checksum_bad_count() -> u64 {
    GPS_NMEA_CHECKSUM_BAD.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_nmea_rmc_count() -> u64 {
    GPS_NMEA_RMC_COUNT.lock().map(|g| *g).unwrap_or(0)
}

pub fn gps_nmea_gga_count() -> u64 {
    GPS_NMEA_GGA_COUNT.lock().map(|g| *g).unwrap_or(0)
}
