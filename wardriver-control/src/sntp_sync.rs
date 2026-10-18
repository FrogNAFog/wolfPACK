//! SNTP wall-clock sync for HTTPS upload windows (control-only).
//!
//! GPS may already set [`time_sync::wall_clock_trusted`] before STA connect; when it has not,
//! poll briefly then run esp-idf SNTP over the home WiFi link.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use esp_idf_svc::sntp::{EspSntp, OperatingMode, SntpConf, SyncMode, SyncStatus};
use log::{info, warn};

use wardriver_shared::task_wdt;

use crate::storage::append_diag_log_line_best_effort;
use crate::time_sync;
use wardriver_stats as stats;

const GPS_WAIT: Duration = Duration::from_secs(30);
const SNTP_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(200);

/// Preferred NTP hostnames (slot 0 first). Array length follows IDF `SNTP_MAX_SERVERS`.
const SNTP_SERVER_PREF: &[&str] = &["pool.ntp.org", "time.google.com"];

const SNTP_SLOT_COUNT: usize = esp_idf_sys::SNTP_MAX_SERVERS as usize;

const fn sntp_servers() -> [&'static str; SNTP_SLOT_COUNT] {
    let mut out: [&'static str; SNTP_SLOT_COUNT] = [SNTP_SERVER_PREF[0]; SNTP_SLOT_COUNT];
    let mut i = 0usize;
    while i < SNTP_SLOT_COUNT {
        out[i] = if i < SNTP_SERVER_PREF.len() {
            SNTP_SERVER_PREF[i]
        } else {
            SNTP_SERVER_PREF[0]
        };
        i += 1;
    }
    out
}

const SNTP_SERVERS: [&str; SNTP_SLOT_COUNT] = sntp_servers();

/// Wait for GPS, then SNTP if needed. Returns `true` when upload may proceed.
pub fn ensure_wall_clock_for_upload(context: &str) -> bool {
    if wait_for_trusted_clock(GPS_WAIT) {
        info!("{context}: wall clock trusted before upload");
        return true;
    }

    info!("{context}: wall clock not set by GPS within {GPS_WAIT:?}; trying SNTP");
    match run_sntp_sync(SNTP_WAIT) {
        Ok(()) => {
            info!("{context}: wall clock trusted after SNTP");
            true
        }
        Err(e) => {
            if time_sync::wall_clock_trusted() {
                info!("{context}: wall clock trusted after SNTP attempt ({e:#})");
                return true;
            }
            warn!("{context}: upload skipped — wall clock not trusted: {e:#}");
            log_upload_clock_skip(context);
            false
        }
    }
}

fn wait_for_trusted_clock(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if time_sync::wall_clock_trusted() {
            return true;
        }
        task_wdt::sleep_feeding_wdt(POLL);
    }
    time_sync::wall_clock_trusted()
}

fn run_sntp_sync(timeout: Duration) -> Result<()> {
    let conf = SntpConf {
        servers: SNTP_SERVERS,
        operating_mode: OperatingMode::Poll,
        sync_mode: SyncMode::Immediate,
    };
    let sntp = EspSntp::new(&conf).context("EspSntp::new")?;

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if sntp.get_sync_status() == SyncStatus::Completed {
            time_sync::apply_sntp_wall_clock();
            if time_sync::wall_clock_trusted() {
                return Ok(());
            }
            anyhow::bail!("SNTP completed but wall clock still untrusted");
        }
        task_wdt::sleep_feeding_wdt(POLL);
    }
    anyhow::bail!("SNTP sync timed out after {timeout:?}")
}

fn log_upload_clock_skip(context: &str) {
    let line = format!(
        "UPLOAD kind=SKIP_CLOCK context={context} utc={} t_ms={} trusted={} free_internal_heap={} free_spiram_heap={}",
        time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
        stats::now_ms(),
        time_sync::wall_clock_trusted(),
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
    );
    append_diag_log_line_best_effort(&line);
}
