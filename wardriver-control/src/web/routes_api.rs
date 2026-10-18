//! JSON API handlers: capture (probe PCAP), channel plan, mode, LPR-camera detectors.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use embedded_io::ErrorType;
use embedded_io::Write as EioWrite;
use embedded_svc::http::server::Connection;
use embedded_svc::http::Method;
use esp_idf_svc::http::server::EspHttpServer;
use log::warn;
use serde_json::json;
use wardriver_stats as stats;

use crate::channel_plan;
use crate::config;
use crate::control::fleet_cmd;
use crate::control::{NvsDeferOp, NVS_DEFER_WAIT_MS};
use crate::fleet_roster;
use crate::recon;
use crate::storage;
use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;

use super::channel_plan_api;
use super::json_escape;
use super::read_post_form;
use super::routes::WebRouteDeps;
use super::snapshots::write_snapshot;
use super::stats_handler;

/// `POST /api/detection-detectors` (+ legacy `/api/flock-detectors`) — merge detector masks into SD.
fn handle_detection_detectors<C>(
    mut req: esp_idf_svc::http::server::Request<C>,
    detection_masks_dirty: &std::sync::atomic::AtomicBool,
) -> Result<(), anyhow::Error>
where
    C: ErrorType + Connection,
{
    let raw = read_post_form(&mut req, 128);
    let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
    let wifi_s = form.get("wifi_disable").map(String::as_str).unwrap_or("0");
    let ble_s = form.get("ble_disable").map(String::as_str).unwrap_or("0");
    let wifi_mask: u32 = wifi_s.parse().unwrap_or(0);
    let ble_mask: u32 = ble_s.parse().unwrap_or(0);
    let ble_active = form
        .get("ble_active_scan")
        .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .unwrap_or(stats::ble_active_scan());
    let json_out = match storage::sd_io_handle() {
        Some(sd_io) => {
            stats::set_detection_disable_masks(wifi_mask, ble_mask);
            stats::set_ble_active_scan(ble_active);
            match crate::preferences::persist_from_stats(sd_io) {
                Ok(()) => {
                    detection_masks_dirty.store(true, Ordering::Release);
                    r#"{"ok":true}"#.to_string()
                }
                Err(e) => format!(
                    "{{\"ok\":false,\"error\":\"{}\"}}",
                    json_escape(&format!("{e:#}"))
                ),
            }
        }
        None => r#"{"ok":false,"error":"SD I/O worker not ready"}"#.to_string(),
    };
    let mut resp = match req.into_ok_response() {
        Ok(r) => r,
        Err(e) => return Err(anyhow!("into_ok_response: {e:?}")),
    };
    resp.write_all(json_out.as_bytes())
        .map_err(|e| anyhow!("write: {e:?}"))?;
    Ok(())
}

/// `POST /api/detection-ignore-mac` (+ legacy `/api/flock-ignore-mac`) — append one MAC to the ignore set.
fn handle_detection_ignore_mac<C>(
    mut req: esp_idf_svc::http::server::Request<C>,
    detection_mac_ignore: &std::sync::Arc<std::sync::RwLock<std::collections::HashSet<[u8; 6]>>>,
) -> Result<(), anyhow::Error>
where
    C: ErrorType + Connection,
{
    let raw = read_post_form(&mut req, 256);
    let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
    let mac_s = form.get("mac").cloned().unwrap_or_default();

    let json_out = if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        r#"{"ok":false,"error":"SD not mounted — cannot persist LPR-camera ignore list"}"#
            .to_string()
    } else if let Some(m) = config::parse_mac_bytes(mac_s.trim()) {
        match detection_mac_ignore.write() {
            Ok(mut g) => {
                g.insert(m);
                let copy = g.clone();
                drop(g);
                let path = storage::flock_camera_false_positive_ignore_path();
                match storage::sd_io_handle().map(|sd_io| {
                    storage::save_mac_ignore_list(
                        sd_io,
                        &path,
                        storage::FLOCK_IGNORE_FILE_HEADER,
                        &copy,
                    )
                }) {
                    Some(Ok(())) => r#"{"ok":true}"#.to_string(),
                    Some(Err(e)) => format!(
                        "{{\"ok\":false,\"error\":\"{}\"}}",
                        json_escape(&format!("{e}"))
                    ),
                    None => r#"{"ok":false,"error":"SD I/O worker not ready"}"#.to_string(),
                }
            }
            Err(_) => r#"{"ok":false,"error":"lock failed"}"#.to_string(),
        }
    } else {
        r#"{"ok":false,"error":"invalid mac"}"#.to_string()
    };

    let mut resp = match req.into_ok_response() {
        Ok(r) => r,
        Err(e) => return Err(anyhow!("into_ok_response: {e:?}")),
    };
    resp.write_all(json_out.as_bytes())
        .map_err(|e| anyhow!("write: {e:?}"))?;
    Ok(())
}

/// Queue WiFi probe PCAP capture when `job` is valid and SD is ready.
fn capture_start_json(job: &str) -> String {
    if job != "wifi_probe_pcap" {
        return format!(
            "{{\"ok\":false,\"error\":\"{}\"}}",
            json_escape("unsupported job (use wifi_probe_pcap)")
        );
    }
    if stats::RECON_ACTIVE.load(Ordering::Acquire) != 0 {
        return r#"{"ok":false,"error":"capture already active"}"#.to_string();
    }
    if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        return r#"{"ok":false,"error":"SD not mounted"}"#.to_string();
    }
    recon::request_wifi_probe_recon();
    r#"{"ok":true}"#.to_string()
}

pub(crate) fn register_api_routes(
    http: &mut EspHttpServer<'static>,
    deps: &WebRouteDeps,
) -> Result<(), anyhow::Error> {
    // GET catalog (capture + legacy recon paths).
    for path in ["/api/capture/catalog", "/api/recon/catalog"] {
        http.fn_handler(path, Method::Get, |req| -> Result<(), anyhow::Error> {
            let mut resp = req.into_ok_response()?;
            resp.write_all(recon::catalog_json().as_bytes())
                .map_err(|e| anyhow!("{e:?}"))
        })?;
    }

    // POST start wifi_probe_pcap (capture + legacy recon paths).
    for path in ["/api/capture/start", "/api/recon/start"] {
        http.fn_handler(path, Method::Post, |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 256);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let job = form.get("job").map(String::as_str).unwrap_or("");
            let json_out = capture_start_json(job);
            let mut resp = req.into_ok_response()?;
            resp.write_all(json_out.as_bytes())
                .map_err(|e| anyhow!("{e:?}"))
        })?;
    }

    let cp_snap = Arc::clone(&deps.web_snapshots);
    http.fn_handler(
        "/api/channel-plan",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let snap = cp_snap
                .read()
                .map_err(|_| anyhow!("snapshot lock poisoned"))?
                .snapshots
                .channel_plan
                .clone();
            write_snapshot(req, &snap)
        },
    )?;

    http.fn_handler(
        "/api/channel-plan/preview",
        Method::Post,
        |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let candidate = channel_plan_api::parse_channel_plan_form(&form);
            let json_out = channel_plan_api::channel_plan_response_json(true, Some(&candidate));
            let mut resp = req.into_ok_response()?;
            resp.write_all(json_out.as_bytes())
                .map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    http.fn_handler(
        "/api/channel-plan",
        Method::Post,
        |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let candidate = channel_plan_api::parse_channel_plan_form(&form);
            let rev = channel_plan::set_plan(candidate.clone());
            let mut warn_msg = String::new();
            if let Some(sd_io) = storage::sd_io_handle() {
                if let Err(e) = channel_plan::save_to_sd(&candidate, sd_io) {
                    warn_msg = format!("{e}");
                    warn!("web: channel plan SD save failed: {e}");
                }
            } else {
                warn_msg = "SD I/O worker not ready".to_string();
            }
            let out = if warn_msg.is_empty() {
                json!({
                    "ok": true,
                    "revision": rev
                })
            } else {
                json!({
                    "ok": true,
                    "revision": rev,
                    "sd_warning": warn_msg
                })
            }
            .to_string();
            let mut resp = req.into_ok_response()?;
            resp.write_all(out.as_bytes())
                .map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    // POST /api/channel-plan/reset — reset plan to factory defaults, clear all band assignments
    // and pins, bump revision so control re-sends schedules. BLE sniffers are not touched
    // (they are not part of the channel plan at all).
    http.fn_handler(
        "/api/channel-plan/reset",
        Method::Post,
        |req| -> Result<(), anyhow::Error> {
            let (rev, sd_saved) = storage::sd_io_handle()
                .map(|sd_io| channel_plan::reset_to_defaults(sd_io))
                .unwrap_or_else(|| {
                    let rev = channel_plan::set_plan(channel_plan::ChannelPlan::default());
                    (rev, false)
                });
            let mut body = format!(
                r#"{{"ok":true,"revision":{},"message":"Reset to defaults""#,
                rev
            );
            if !sd_saved {
                let warn_msg = if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
                    "SD not mounted — defaults apply in RAM until SD is available"
                } else {
                    "SD save failed — defaults apply in RAM until save succeeds"
                };
                body.push_str(",\"sd_warning\":\"");
                body.push_str(&json_escape(warn_msg));
                body.push('"');
            }
            body.push('}');
            let mut resp = req.into_ok_response()?;
            resp.write_all(body.as_bytes())
                .map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    http.fn_handler(
        "/sniffers/role",
        Method::Post,
        |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 512);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let sid = form.get("sniffer_id").and_then(|s| s.parse::<u32>().ok());
            let role_s = form.get("role").map(String::as_str).unwrap_or("wifi");
            if role_s == "i154" || role_s == "802154" || role_s == "2" {
                let mut resp = req.into_response(
                    400,
                    Some("Bad Request"),
                    &[("Content-Type", "application/json")],
                )?;
                resp.write_all(br#"{"ok":false,"code":"ROLE_REMOVED"}"#)
                    .map_err(|e| anyhow!("{e:?}"))?;
                return Ok(());
            }
            let role_wire: u8 = if role_s == "ble" || role_s == "1" {
                1
            } else {
                0
            };
            let msg: &str = if let Some(id) = sid {
                if let Some(m) = stats::mac_for_sniffer_id(id) {
                    stats::enqueue_role_change(id, m, role_wire);
                    if role_wire == 1 {
                        let fleet_slot = stats::fleet_slot_for_sniffer(id);
                        stats::enqueue_deferred_welcome(id, m, fleet_slot, true);
                    }
                    "Queued SetRole for next control tick."
                } else {
                    "Unknown sniffer_id (not seen yet)."
                }
            } else {
                "Missing sniffer_id."
            };
            let mut resp = req.into_ok_response()?;
            EioWrite::write_all(&mut resp, msg.as_bytes()).map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    http.fn_handler("/api/mode", Method::Post, |mut req| {
        let raw = read_post_form(&mut req, 128);
        let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
        let mode = form.get("mode").map(String::as_str).unwrap_or("");
        let v = if mode == "management" {
            stats::MODE_REQ_MANAGEMENT
        } else if mode == "wardrive" {
            stats::MODE_REQ_WARDRIVE
        } else {
            return Err(anyhow!("expected mode=wardrive or mode=management"));
        };
        stats::MODE_REQUEST.store(v, std::sync::atomic::Ordering::Release);
        let mut resp = req.into_ok_response()?;
        resp.write_all(br#"{"ok":true}"#)
            .map_err(|e| anyhow!("{e:?}"))?;
        Ok(())
    })?;

    http.fn_handler(
        "/api/flock",
        Method::Post,
        |mut req| -> Result<(), anyhow::Error> {
            let _raw = read_post_form(&mut req, 128);
            let mut resp = req.into_response(
                404,
                Some("Not Found"),
                &[("Content-Type", "application/json")],
            )?;
            resp.write_all(br#"{"ok":false,"code":"DEPRECATED_USE_FLOCK_DETECTORS"}"#)
                .map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    http.fn_handler(
        "/api/axon",
        Method::Post,
        |mut req| -> Result<(), anyhow::Error> {
            let _raw = read_post_form(&mut req, 128);
            let mut resp = req.into_response(
                404,
                Some("Not Found"),
                &[("Content-Type", "application/json")],
            )?;
            resp.write_all(br#"{"ok":false,"code":"DEPRECATED_AXON_IS_BLE_METHOD_35"}"#)
                .map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    let detection_masks_dirty = Arc::clone(&deps.detection_masks_dirty);
    http.fn_handler(
        "/api/detection-detectors",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_detectors(req, &detection_masks_dirty)
        },
    )?;

    // Legacy alias: same handler as `/api/detection-detectors`.
    let detection_masks_dirty_alias = Arc::clone(&deps.detection_masks_dirty);
    http.fn_handler(
        "/api/flock-detectors",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_detectors(req, &detection_masks_dirty_alias)
        },
    )?;

    let detection_mac_ignore = Arc::clone(&deps.detection_exclude_manual);
    http.fn_handler(
        "/api/detection-ignore-mac",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_ignore_mac(req, &detection_mac_ignore)
        },
    )?;

    // Legacy alias: same handler as `/api/detection-ignore-mac`.
    let detection_mac_ignore_alias = Arc::clone(&deps.detection_exclude_manual);
    http.fn_handler(
        "/api/flock-ignore-mac",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_ignore_mac(req, &detection_mac_ignore_alias)
        },
    )?;

    let nvs_defer_adopt = deps.nvs_defer.clone();
    http.fn_handler(
        "/sniffers/adopt",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 512);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let sid = form.get("sniffer_id").and_then(|s| s.parse::<u32>().ok());
            let msg: String = if let Some(id) = sid {
                let mac = stats::mac_for_sniffer_id(id);
                if let Some(mac) = mac {
                    let fleet_slot = match fleet_roster::with_roster_mut(|r| r.adopt(id, mac)) {
                        Some(Ok(slot)) => slot,
                        Some(Err(e)) => return Err(e),
                        None => return Err(anyhow!("fleet roster not initialized")),
                    };
                    let blob = fleet_roster::roster_blob_snapshot()?;
                    nvs_defer_adopt
                        .submit_and_wait(
                            NvsDeferOp::FleetRosterWriteBlob(blob),
                            Duration::from_millis(NVS_DEFER_WAIT_MS),
                        )
                        .map_err(|e| anyhow!(e))?;
                    stats::set_adoption_adopted(id, fleet_slot);
                    stats::enqueue_adopt_welcome(id, mac, fleet_slot);
                    format!("Adopted fleet_slot={fleet_slot}; welcome queued.")
                } else {
                    "Unknown sniffer_id (not seen yet).".to_string()
                }
            } else {
                "Missing sniffer_id.".to_string()
            };
            let mut resp = req.into_ok_response()?;
            EioWrite::write_all(&mut resp, msg.as_bytes()).map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    let nvs_defer_remove = deps.nvs_defer.clone();
    http.fn_handler(
        "/sniffers/remove",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 512);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let sid = form.get("sniffer_id").and_then(|s| s.parse::<u32>().ok());
            let msg: &str = if let Some(id) = sid {
                let mac = stats::mac_for_sniffer_id(id);
                let removed = fleet_roster::with_roster_mut(|r| r.remove(id)).unwrap_or(false);
                if removed {
                    if let Ok(blob) = fleet_roster::roster_blob_snapshot() {
                        let _ = nvs_defer_remove.submit_and_wait(
                            NvsDeferOp::FleetRosterWriteBlob(blob),
                            Duration::from_millis(NVS_DEFER_WAIT_MS),
                        );
                    }
                    stats::set_adoption_removed(id);
                    if let Some(mac) = mac {
                        let release_epoch = stats::now_ms() as u32;
                        if let Err(e) = fleet_cmd::send_release_binding_unicast(
                            mac,
                            release_epoch,
                            ESPNOW_PEER_IF,
                        ) {
                            warn!("web: remove release_binding sniffer_id={id}: {e:#}");
                        }
                    }
                    "Removed from roster."
                } else {
                    "Unknown sniffer_id."
                }
            } else {
                "Missing sniffer_id."
            };
            let mut resp = req.into_ok_response()?;
            EioWrite::write_all(&mut resp, msg.as_bytes()).map_err(|e| anyhow!("{e:?}"))?;
            Ok(())
        },
    )?;

    let ui_forms_snap = Arc::clone(&deps.web_snapshots);
    http.fn_handler(
        "/api/ui-forms",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            let snap = ui_forms_snap
                .read()
                .map_err(|_| anyhow!("snapshot lock poisoned"))?
                .snapshots
                .ui_forms
                .clone();
            write_snapshot(req, &snap)
        },
    )?;

    stats_handler::register(http, Arc::clone(&deps.web_snapshots))?;

    Ok(())
}
