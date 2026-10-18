//! LPR-camera WiFi probe-request burst ESP-NOW handler (telemetry + follow-up rows only).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use crate::csv;
use crate::detection_alert_log::DetectionAlertLog;
use crate::gps;
use crate::storage;
use wardriver_proto::{ProbeReqBurstV1, ProbeReqFragV1Header};
use wardriver_stats as stats;

use super::csv_ingest::{
    detection_append_wifi_followup_detection, detection_fix_for_observation, empty_bssid_set,
};
use super::ingress::sync_adoption_from_roster;
use super::ingress_ctx::ControlIngressCtx;
use wardriver_core::gps_anchor::observed_unix_ms_from_anchor;

/// WiFi LPR-camera follow-up rows for probe bursts (ESP-NOW frag or HTTP dump batch).
pub(crate) fn apply_probe_burst_followups(
    hdr: &ProbeReqFragV1Header,
    entries: &[ProbeReqBurstV1],
    detection_ignore: &HashSet<[u8; 6]>,
    gps_hist: &VecDeque<(i64, gps::Fix)>,
    gps_fix: &Arc<Mutex<Option<gps::Fix>>>,
    detection_alert_log_wifi: &mut DetectionAlertLog,
    mac_table: &mut csv::MacTable,
    sd_io: &storage::SdIoHandle,
) -> u32 {
    let mut written = 0u32;
    for e in entries {
        if detection_ignore.contains(&e.src) {
            continue;
        }
        if let Some(obs_unix) = observed_unix_ms_from_anchor(
            hdr.sniffer_scan_end_ms,
            hdr.sniffer_scan_end_unix_ms,
            e.first_seen_ms32,
        ) {
            if let Some(fix) = detection_fix_for_observation(gps_hist, gps_fix, Some(obs_unix)) {
                if let Some(meta) =
                    detection_alert_log_wifi.try_followup(&e.src, fix.lat, fix.lon, mac_table)
                {
                    if detection_append_wifi_followup_detection(
                        hdr,
                        e,
                        &meta,
                        &fix,
                        Some(obs_unix),
                        sd_io,
                    ) {
                        written += 1;
                    }
                }
            }
        }
    }
    written
}

pub(super) fn handle_probe_req_frag(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    stats::DETECTION_PROBE_FRAGS_RX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let Ok((hdr, entries)) = wardriver_proto::unpack_probe_req_frag(data) else {
        return;
    };
    stats::sniffer_seen_liveness(hdr.sniffer_id, &peer);
    sync_adoption_from_roster(hdr.sniffer_id, &peer);
    super::peer_pool::note_bulk_frag(peer, stats::now_ms());
    stats::DETECTION_BURSTS_RX
        .fetch_add(entries.len() as u32, std::sync::atomic::Ordering::Relaxed);
    let detection_ignore = ctx.detection_ignore.read().ok();
    let detection_ignore_ref: &HashSet<[u8; 6]> = match detection_ignore.as_deref() {
        Some(s) => s,
        None => empty_bssid_set(),
    };
    apply_probe_burst_followups(
        &hdr,
        &entries,
        detection_ignore_ref,
        ctx.gps_hist,
        ctx.gps_fix,
        ctx.detection_alert_log_wifi,
        ctx.mac_table,
        ctx.sd_io,
    );
    if hdr.flags & wardriver_proto::FRAG_FLAG_TRANSPORT_END != 0 {
        super::ingress::run_post_dump_finish(peer, ctx);
    }
}
