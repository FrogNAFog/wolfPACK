//! Wall-clock synchronization from GPS NMEA (RMC date/time + quality gating elsewhere)
//! and SNTP during HTTPS upload windows.
//!
//! The ESP32-C5 devkit has no battery-backed RTC, so the clock resets every
//! boot. Until this module flips [`TIME_VALID`] true, file creation on the SD
//! card is deferred to avoid 1970-dated FAT entries.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::Result;
use esp_idf_sys as sys;
use log::{info, warn};

use wardriver_stats as stats;

pub static TIME_VALID: AtomicBool = AtomicBool::new(false);

/// Control `stats::now_ms()` when [`TIME_VALID`] last transitioned false → true.
///
/// Post-dump RTT treats any sniffer whose `last_rtt_time_set_ms` is older than this as due,
/// so fleet members connected during invalid wall clock get a fresh RTT after GPS/SNTP.
static WALL_CLOCK_TRUSTED_SINCE_MS: Mutex<u64> = Mutex::new(0);

/// True after GPS or SNTP has applied a trusted UTC wall clock (`TIME_VALID`).
///
/// Use this anywhere the control node must agree with SD session / WiGLE row policy
/// (same predicate historically spelled `time_ok` in the main loop).
#[inline]
pub fn wall_clock_trusted() -> bool {
    TIME_VALID.load(Ordering::Acquire)
}

/// Monotonic ms when wall clock last became trusted; `0` if never yet.
#[inline]
pub fn wall_clock_trusted_since_ms() -> u64 {
    WALL_CLOCK_TRUSTED_SINCE_MS.lock().map(|g| *g).unwrap_or(0)
}

static GPS_SYNCED: AtomicBool = AtomicBool::new(false);
static SNTP_SYNCED: AtomicBool = AtomicBool::new(false);

/// Unix ms for 2020-01-01 00:00:00 UTC — reject SNTP/GPS times still near epoch.
const MIN_TRUSTED_WALL_MS: i64 = 1_577_836_800_000;

/// Last successful GPS application, for throttling further resyncs.
static LAST_GPS_APPLY_MS: Mutex<i64> = Mutex::new(i64::MIN);

const GPS_RESYNC_MIN_DELTA_MS: i64 = 2_000;
const GPS_RESYNC_MIN_INTERVAL_MS: i64 = 60_000;

/// Set the process timezone to UTC so `localtime_r` (used by FAT for file
/// timestamps) agrees with GPS-derived UTC.
pub fn set_tz_utc() {
    unsafe {
        let k = b"TZ\0".as_ptr() as *const _;
        let v = b"UTC0\0".as_ptr() as *const _;
        let _setenv_rc = sys::setenv(k, v, 1);
        // ESP32-C5/newlib currently faults inside tzset() while parsing "UTC0".
        // Leaving TZ in the environment avoids the boot crash; UTC conversion
        // paths in this app use explicit epoch math rather than localtime.
        // sys::tzset();
    }
    info!("time_sync: TZ=UTC0 applied");
}

/// Apply GPS-derived UTC to the system clock. Called from the NMEA reader on
/// every RMC with status=A and a parseable date.
pub fn apply_gps_utc(year: u16, month: u8, day: u8, h: u8, m: u8, s: u8, ms: u16) {
    let Some(unix_secs) = ymd_hms_to_unix(year, month, day, h, m, s) else {
        warn!("time_sync: invalid GPS date {year:04}-{month:02}-{day:02}");
        return;
    };
    let gps_ms = unix_secs * 1000 + ms as i64;
    let now_ms = system_now_ms();
    let delta_ms = (gps_ms - now_ms).abs();

    let last = LAST_GPS_APPLY_MS.lock().map(|g| *g).unwrap_or(i64::MIN);
    let since_last = now_ms.saturating_sub(last);

    if wall_clock_trusted()
        && (delta_ms < GPS_RESYNC_MIN_DELTA_MS || since_last < GPS_RESYNC_MIN_INTERVAL_MS)
    {
        return;
    }

    if let Err(e) = set_system_ms(gps_ms) {
        warn!("time_sync: settimeofday failed: {e}");
        return;
    }
    if let Ok(mut g) = LAST_GPS_APPLY_MS.lock() {
        *g = now_ms;
    }

    if !GPS_SYNCED.load(Ordering::Acquire) {
        info!(
            "time_sync: GPS time set ({year:04}-{month:02}-{day:02} {h:02}:{m:02}:{s:02}.{ms:03}Z), delta={delta_ms} ms"
        );
        on_first_sync();
    } else {
        info!("time_sync: GPS drift correction applied, delta={delta_ms} ms");
    }
}

fn on_first_sync() {
    mark_wall_clock_trusted(system_now_ms().max(0) as u64);
    wardriver_stats::set_gps_time_valid(true);
    if !GPS_SYNCED.swap(true, Ordering::AcqRel) {
        info!("time_sync: first sync from GPS");
    }
}

/// Flip [`TIME_VALID`] and reset per-sniffer RTT intervals on the first false → true edge.
fn mark_wall_clock_trusted(_system_now_ms: u64) {
    let was_trusted = TIME_VALID.swap(true, Ordering::AcqRel);
    if was_trusted {
        return;
    }
    // Align with `stats::sniffer_last_rtt_time_set_ms` / post-dump RTT deadlines.
    let mono_ms = stats::now_ms();
    if let Ok(mut g) = WALL_CLOCK_TRUSTED_SINCE_MS.lock() {
        *g = mono_ms;
    }
    // Sniffers may have recorded a no-op RTT while control time was still untrusted.
    stats::clear_all_sniffer_last_rtt_time_set_ms();
    info!("time_sync: wall clock trusted; sniffer RTT due markers reset");
}

/// Mark wall clock trusted after esp-idf SNTP has updated system time.
///
/// Does not set [`wardriver_stats::set_gps_time_valid`]; GPS quality gates stay separate.
pub fn apply_sntp_wall_clock() {
    let now_ms = system_now_ms();
    if now_ms < MIN_TRUSTED_WALL_MS {
        warn!("time_sync: SNTP system time {now_ms} ms still before 2020, not trusting");
        return;
    }
    if wall_clock_trusted() {
        return;
    }
    mark_wall_clock_trusted(now_ms.max(0) as u64);
    if !SNTP_SYNCED.swap(true, Ordering::AcqRel) {
        info!("time_sync: first sync from SNTP");
    }
}

fn system_now_ms() -> i64 {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, std::ptr::null_mut());
    }
    (tv.tv_sec as i64) * 1000 + (tv.tv_usec as i64) / 1000
}

fn set_system_ms(ms: i64) -> Result<()> {
    let tv = sys::timeval {
        tv_sec: (ms / 1000) as sys::time_t,
        tv_usec: ((ms % 1000) * 1000) as sys::suseconds_t,
    };
    let rc = unsafe { sys::settimeofday(&tv, std::ptr::null()) };
    if rc != 0 {
        anyhow::bail!("settimeofday rc={rc}");
    }
    Ok(())
}

/// Days-from-civil algorithm (Howard Hinnant); accepts Gregorian y/m/d and
/// returns the Unix epoch seconds for 00:00:00 UTC on that date.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as i64) * 146_097 + doe as i64 - 719_468
}

fn ymd_hms_to_unix(year: u16, month: u8, day: u8, h: u8, m: u8, s: u8) -> Option<i64> {
    if !(1970..=2099).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || h > 23
        || m > 59
        || s > 60
    {
        return None;
    }
    let d = days_from_civil(year as i32, month as u32, day as u32);
    Some(d * 86_400 + (h as i64) * 3600 + (m as i64) * 60 + (s as i64))
}

/// Format the current system time as `YYYY-MM-DD hh:mm:ss` UTC for CSV rows.
/// Returns `None` when [`TIME_VALID`] has not been asserted yet.
pub fn now_utc_string() -> Option<String> {
    if !wall_clock_trusted() {
        return None;
    }
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, std::ptr::null_mut());
    }
    let t = tv.tv_sec as i64;
    let days = t.div_euclid(86_400);
    let tod = t.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = (tod / 3600) as u32;
    let mi = ((tod % 3600) / 60) as u32;
    let s = (tod % 60) as u32;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, mo, d, h, mi, s
    ))
}

/// Format a Unix timestamp (milliseconds) as `YYYY-MM-DD hh:mm:ss` UTC.
///
/// Unlike [`now_utc_string`], this does **not** depend on [`TIME_VALID`]; it is intended
/// for using a GPS-provided `Fix.utc_unix_ms` directly when aligning observations to RF time.
pub fn utc_string_from_unix_ms(unix_ms: i64) -> Option<String> {
    let t = unix_ms.div_euclid(1000);
    let days = t.div_euclid(86_400);
    let tod = t.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = (tod / 3600) as u32;
    let mi = ((tod % 3600) / 60) as u32;
    let s = (tod % 60) as u32;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, mo, d, h, mi, s
    ))
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + (era as i32) * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
