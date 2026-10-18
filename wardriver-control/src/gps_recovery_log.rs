//! SD diag logging hook for GPS recovery counters in [`wardriver_stats`].

use wardriver_stats as stats;

pub fn install() {
    stats::install_gps_recovery_logger(log_gps_recovery_to_sd);
}

fn log_gps_recovery_to_sd(kind: &str, detail: &str) {
    let now_ms = stats::now_ms();
    let line = format!(
        "GPS_RECOVERY kind={kind} utc={} t_ms={now_ms} rmc_age_ms={} uart_idle_ms={} {detail}",
        crate::time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
        stats::gps_rmc_last_parse_age_ms(now_ms).min(u32::MAX as u64),
        stats::gps_uart_rx_age_ms(now_ms).min(u32::MAX as u64),
    );
    if let Some(sd_io) = crate::storage::sd_io_handle() {
        let _ = sd_io.try_enqueue_diag_line(line);
    }
}
