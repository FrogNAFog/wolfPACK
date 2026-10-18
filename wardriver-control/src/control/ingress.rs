//! ESP-NOW frame dispatch: sniffer hello/ack, detection alerts, probe bursts, scan fragments, recon.

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use log::warn;

use crate::fleet_roster;
use crate::recon;
use wardriver_proto::{self as proto, FrameType};
use wardriver_shared::espnow;
use wardriver_stats as stats;

use super::csv_ingest::{
    empty_bssid_set, process_completed_ble_reassembly, wifi_enqueue_csv_for_batch,
};
use super::csv_ingest_probe::probe_enqueue_csv_for_batch;
use super::espnow_rx;
use super::ingress_detection_alert;
use super::ingress_detection_ble_alert;
use super::ingress_detection_probe;
use super::ingress_remote_id_alert;
use super::ingress_sniffer::{self, uplink_liveness_and_boot_ok};
use super::ingress_ssid_watch;

pub(crate) use super::ingress_ctx::ControlIngressCtx;

/// Detection-ignore set for this tick: the live `detection_ignore` set, or a shared empty set
/// when the `RwLock` is poisoned. The caller reads the guard (so it outlives the use site) and
/// passes the deref; this replaces the repeated `match`/`empty_bssid_set` dance.
fn detection_ignore_or_empty(g: Option<&HashSet<[u8; 6]>>) -> &HashSet<[u8; 6]> {
    g.unwrap_or(empty_bssid_set())
}

fn maybe_post_dump_grace(mac: [u8; 6], flags: u8, ctx: &ControlIngressCtx<'_>) {
    if flags & proto::FRAG_FLAG_TRANSPORT_END == 0 {
        return;
    }
    run_post_dump_finish(mac, ctx);
}

fn note_bulk_frag(mac: [u8; 6]) {
    super::peer_pool::note_bulk_frag(mac, stats::now_ms());
}

pub(super) fn run_post_dump_finish(mac: [u8; 6], _ctx: &ControlIngressCtx<'_>) {
    let now = stats::now_ms();
    super::peer_pool::mark_transport_end(mac, now);
    super::peer_pool::enqueue_post_dump_flush(mac);
}

/// Roster gate: adopted sniffers must match NVS `(sniffer_id, mac)`.
fn roster_allows(sniffer_id: u32, mac: &[u8; 6]) -> bool {
    fleet_roster::with_roster(|r| r.is_adopted(sniffer_id, mac)).unwrap_or(false)
}

/// If NVS roster lists this peer, mirror Adopted + roster `fleet_slot` into stats.
pub(super) fn sync_adoption_from_roster(sniffer_id: u32, mac: &[u8; 6]) {
    if let Some(Some(fleet_slot)) = fleet_roster::with_roster(|r| {
        if r.is_adopted(sniffer_id, mac) {
            r.fleet_slot_for(sniffer_id)
        } else {
            None
        }
    }) {
        stats::set_adoption_adopted(sniffer_id, fleet_slot);
    }
}

use wardriver_fleet_realtime::frame_router::peek_ingress_sniffer_id;
use wardriver_fleet_realtime::ingress_roster_should_drop;

fn dispatch_one_frame(ctx: &mut ControlIngressCtx<'_>, frame: espnow::RxFrame) {
    stats::queue_inflight_dec(&stats::ESPNOW_RX_QUEUE_INFLIGHT);
    let data = frame.payload();
    if let Ok(mut s) = ctx.sniffers.lock() {
        s.insert(frame.src);
    }

    if data.len() < 2 || data[0] != proto::PROTO_VERSION {
        if data.len() >= 1 && data[0] != proto::PROTO_VERSION {
            stats::INGRESS_PROTO_REJECT.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }

    if let Some(sid) = peek_ingress_sniffer_id(data) {
        let allows = roster_allows(sid, &frame.src);
        if ingress_roster_should_drop(data[1], allows) {
            stats::INGRESS_ROSTER_DROP.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }

    match data[1] {
        x if x == FrameType::SnifferHello as u8 => {
            stats::INGRESS_HELLO_RX.fetch_add(1, Ordering::Relaxed);
            ingress_sniffer::handle_sniffer_hello(ctx, frame.src, data);
        }
        x if x == FrameType::PeerJoin as u8 => {
            ingress_sniffer::handle_peer_join(ctx, frame.src, data);
        }
        x if x == FrameType::SnifferAck as u8 => {
            ingress_sniffer::handle_sniffer_ack(ctx, frame.src, data);
        }
        x if x == FrameType::TimeSyncPong as u8 => {
            use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;
            if let Ok(pong) = proto::unpack_time_sync_pong(data) {
                stats::sniffer_seen_liveness(pong.sniffer_id, &frame.src);
                super::time_sync_rtt::handle_time_sync_pong(
                    frame.src,
                    pong,
                    ctx.now_ms,
                    ESPNOW_PEER_IF,
                );
            }
        }
        x if x == FrameType::DetectionAlert as u8 => {
            ingress_detection_alert::handle_detection_alert(ctx, frame.src, data);
        }
        x if x == FrameType::DetectionBleAlert as u8 => {
            ingress_detection_ble_alert::handle_detection_ble_alert(ctx, frame.src, data);
        }
        x if x == FrameType::SsidWatchAlert as u8 => {
            ingress_ssid_watch::handle_ssid_watch_alert(ctx, frame.src, data);
        }
        x if x == FrameType::RemoteIdAlert as u8 => {
            ingress_remote_id_alert::handle_remote_id_alert(ctx, frame.src, data);
        }
        x if x == FrameType::ProbeReqFrag as u8 => {
            ingress_detection_probe::handle_probe_req_frag(ctx, frame.src, data);
        }
        x if x == FrameType::ScanFrag as u8 => {
            handle_wifi_scan_frag(ctx, &frame, data);
        }
        x if x == FrameType::BleScanFrag as u8 => {
            handle_ble_scan_frag(ctx, &frame, data);
        }
        x if x == FrameType::ProbeObservationFrag as u8 => {
            stats::CONTROL_STAGE_INGRESS.fetch_add(1, Ordering::Relaxed);
            let Ok((hdr, entries)) = proto::unpack_probe_observation_frag(data) else {
                stats::INGRESS_UNPACK_FAIL_PROBE.fetch_add(1, Ordering::Relaxed);
                return;
            };
            if !uplink_liveness_and_boot_ok(ctx, hdr.sniffer_id, &frame.src, hdr.sniffer_boot_id) {
                return;
            }
            stats::sniffer_probe_frame(hdr.sniffer_id, entries.len());
            note_bulk_frag(frame.src);
            let now = stats::now_ms();
            if let Some((hdr, entries)) = espnow_rx::ingest_probe_observation_frag(
                hdr,
                entries,
                ctx.dedup_probe,
                ctx.reasm_probe,
                now,
            ) {
                let transport_flags = hdr.flags;
                let _ = probe_enqueue_csv_for_batch(&hdr, &entries, ctx.gps_hist, ctx.sd_io);
                maybe_post_dump_grace(frame.src, transport_flags, ctx);
            }
        }
        x if x == FrameType::ReconWifiFrag as u8 => {
            if let Err(e) = recon::ingest_wifi_frag(ctx.sd_io, data, ctx.last_sniffer_boot_id) {
                warn!("control: recon frag: {e:#}");
            }
        }
        _ => {}
    }
}

/// ScanFrag arm of `dispatch_one_frame`: unpack, liveness gate, reassembly, follow-ups + CSV.
fn handle_wifi_scan_frag(ctx: &mut ControlIngressCtx<'_>, frame: &espnow::RxFrame, data: &[u8]) {
    stats::CONTROL_STAGE_INGRESS.fetch_add(1, Ordering::Relaxed);
    let Ok((hdr, aps, extras)) = proto::unpack_scan_frag_with_extras(data) else {
        stats::INGRESS_UNPACK_FAIL_WIFI.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !uplink_liveness_and_boot_ok(ctx, hdr.sniffer_id, &frame.src, hdr.sniffer_boot_id) {
        return;
    }
    stats::sniffer_frame(hdr.sniffer_id, aps.len());
    note_bulk_frag(frame.src);
    let now = stats::now_ms();
    if let Some((hdr, aps, extras)) =
        espnow_rx::ingest_wifi_scan_frag_with_extras(hdr, aps, extras, ctx.dedup, ctx.reasm, now)
    {
        let transport_flags = hdr.flags;
        let detection_ignore_g = ctx.detection_ignore.read().ok();
        let detection_ignore_ref = detection_ignore_or_empty(detection_ignore_g.as_deref());
        let _ = super::csv_ingest::apply_wifi_ap_ms_followups(
            &hdr,
            &aps,
            detection_ignore_ref,
            ctx.gps_hist,
            ctx.gps_fix,
            ctx.detection_alert_log_wifi,
            ctx.mac_table,
            ctx.sd_io,
        );
        let _ = wifi_enqueue_csv_for_batch(
            hdr,
            aps,
            extras,
            &ctx.own_management_ap_bssid,
            ctx.wigle_home,
            ctx.wigle_manual,
            ctx.wigle_ssids_manual,
            ctx.gps_hist,
            ctx.sd_io,
            ctx.mac_table,
        );
        maybe_post_dump_grace(frame.src, transport_flags, ctx);
    }
}

/// BleScanFrag arm of `dispatch_one_frame`: unpack, liveness gate, reassembly, CSV + alerts.
fn handle_ble_scan_frag(ctx: &mut ControlIngressCtx<'_>, frame: &espnow::RxFrame, data: &[u8]) {
    stats::CONTROL_STAGE_INGRESS.fetch_add(1, Ordering::Relaxed);
    let Ok((hdr, entries, extras)) = proto::unpack_ble_scan_frag_with_extras(data) else {
        stats::INGRESS_UNPACK_FAIL_BLE.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if !uplink_liveness_and_boot_ok(ctx, hdr.sniffer_id, &frame.src, hdr.sniffer_boot_id) {
        return;
    }
    stats::sniffer_ble_frame(hdr.sniffer_id, entries.len());
    note_bulk_frag(frame.src);
    let now = stats::now_ms();
    if let Some((hdr, entry)) = espnow_rx::ingest_ble_scan_frag_with_extras(
        hdr,
        entries,
        extras,
        ctx.dedup_ble,
        ctx.reasm_ble,
        now,
    ) {
        stats::inc_scan_cycle();
        let transport_flags = hdr.flags;
        let detection_ignore_g = ctx.detection_ignore.read().ok();
        let detection_ignore_ref = detection_ignore_or_empty(detection_ignore_g.as_deref());
        process_completed_ble_reassembly(
            hdr,
            entry,
            detection_ignore_ref,
            ctx.now_ms,
            ctx.lpr_camera_ble,
            ctx.sd_io,
            ctx.gps_hist,
            ctx.gps_fix,
            ctx.detection_dis_b,
            ctx.mac_table,
            ctx.detection_alert_log_ble,
            ctx.alert_hub,
        );
        maybe_post_dump_grace(frame.src, transport_flags, ctx);
    }
}

fn is_priority_alert_frame(frame: &espnow::RxFrame) -> bool {
    let p = frame.payload();
    if p.len() < 2 || p[0] != proto::PROTO_VERSION {
        return false;
    }
    matches!(
        p[1],
        x if x == FrameType::DetectionAlert as u8 || x == FrameType::RemoteIdAlert as u8
    )
}

/// Dispatch one batch of ESP-NOW RX frames (priority alerts first, then all other types).
pub(crate) fn process_espnow_frames(ctx: &mut ControlIngressCtx<'_>, frames: Vec<espnow::RxFrame>) {
    for frame in &frames {
        if is_priority_alert_frame(frame) {
            dispatch_one_frame(ctx, frame.clone());
        }
    }
    for frame in frames {
        if !is_priority_alert_frame(&frame) {
            dispatch_one_frame(ctx, frame);
        }
    }
}
