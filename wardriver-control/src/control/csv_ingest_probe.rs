//! Probe observation WiGLE CSV row encoding.
//!
//! Probe requests are stored as WiGLE CSV rows with `Type="PROBE_REQ"` to distinguish
//! them from AP beacon rows. The SSID field is the SSID being probed (not the BSSID),
//! and the MAC is the client's source MAC.

use std::collections::VecDeque;

use crate::csv;
use crate::gps;
use crate::storage;
use wardriver_core::wigle::format_wifi_csv_line;
use wardriver_proto::{ProbeObservationFragV1Header, ProbeObservationWire};
use wardriver_stats as stats;

use super::csv_ingest_gps::match_gps_fix_for_obs;
use wardriver_core::gps_anchor::{observed_unix_ms_from_anchor, MAX_FIX_TIMELINE_DELTA_MS};

/// Probe batch → WiGLE CSV rows on SD (ESP-NOW reassembly complete).
pub(crate) fn probe_enqueue_csv_for_batch(
    hdr: &ProbeObservationFragV1Header,
    entries: &[ProbeObservationWire],
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    sd_io: &storage::SdIoHandle,
) -> Result<(), String> {
    if hdr.sniffer_scan_end_unix_ms == 0 {
        return Ok(());
    }
    let mut batch_lines: Vec<Vec<u8>> = Vec::new();
    for entry in entries {
        let Some(obs_unix) = observed_unix_ms_from_anchor(
            hdr.sniffer_scan_end_ms,
            hdr.sniffer_scan_end_unix_ms,
            hdr.sniffer_scan_end_ms, // batch-level timestamp for all entries
        ) else {
            continue;
        };
        let Some(fix) = match_gps_fix_for_obs(gps_hist, obs_unix, MAX_FIX_TIMELINE_DELTA_MS) else {
            continue;
        };
        let ssid = String::from_utf8_lossy(&entry.ssid[..entry.ssid_len as usize])
            .trim()
            .to_string();
        if ssid.is_empty() {
            continue;
        }
        let mac_str = format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            entry.src_mac[0],
            entry.src_mac[1],
            entry.src_mac[2],
            entry.src_mac[3],
            entry.src_mac[4],
            entry.src_mac[5],
        );
        let freq = csv::channel_to_frequency_mhz(entry.channel);
        let rssi = entry.rssi;
        let alt_m = fix.altitude_m.map(|a| a.round() as i32);
        let acc_m = fix.hdop.map(|h| h as f32 * 10.0);
        let line_inp = wardriver_core::wigle::WigleWifiLineInput {
            bssid: [0u8; 6], // Probe MAC is in the MAC column, not BSSID
            ssid,
            auth_mode: String::new(), // No auth mode for probe requests
            first_seen_utc: format!("{:.3}", obs_unix as f64 / 1000.0),
            channel: entry.channel,
            frequency_mhz: freq,
            rssi,
            lat: Some(fix.lat),
            lon: Some(fix.lon),
            altitude_m: alt_m,
            accuracy_m: acc_m,
            rcois: String::new(),
            mfgr_id: String::new(),
            row_type: "PROBE_REQ",
        };
        let csv_line = format_wifi_csv_line(&line_inp);
        let mut line = Vec::<u8>::with_capacity(csv_line.len() + 16);
        line.extend_from_slice(csv_line.as_bytes());
        line.push(b'\n');
        batch_lines.push(line);
    }
    if batch_lines.is_empty() {
        return Ok(());
    }
    let n = batch_lines.len() as u32;
    if sd_io.try_enqueue_wigle_rows(batch_lines) {
        stats::sniffer_wifi_rows_enqueued(hdr.sniffer_id, n as usize);
    }
    Ok(())
}
