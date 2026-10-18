//! Provisioning POST handlers and SD-backed MAC filter saves (GET pages are static).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::anyhow;
use embedded_io::Write as _;
use embedded_svc::http::Method;
use esp_idf_svc::http::server::EspHttpServer;
use wardriver_stats as stats;

use crate::config;
use crate::control::{NvsDeferOp, NvsDeferQueue, NVS_DEFER_WAIT_MS};
use crate::gps;
use crate::storage;

use super::read_post_form;
use super::redirect::{query_param_escape, see_other};
use super::snapshots::web_snapshots_bump_forms_rev;

fn redirect_home_geo_result(result: Result<(), String>) -> String {
    match result {
        Ok(()) => "/setup?msg=ok".to_string(),
        Err(e) => format!("/setup?msg=err&detail={}", query_param_escape(&e)),
    }
}

/// `POST /detection-ignore/save` (+ legacy `/flock-ignore/save`) — persist MAC ignore list to SD.
fn handle_detection_ignore_save<C>(
    mut req: esp_idf_svc::http::server::Request<C>,
    detection_ex_s: &Arc<RwLock<HashSet<[u8; 6]>>>,
) -> Result<(), anyhow::Error>
where
    C: embedded_io::ErrorType + embedded_svc::http::server::Connection,
{
    let raw = read_post_form(&mut req, 2048);
    let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
    let macs_raw = form.get("macs").cloned().unwrap_or_default();

    let location = if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        "/detections?msg=err&detail=SD%20card%20must%20be%20mounted".to_string()
    } else {
        match storage::parse_mac_ignore_file_text(&macs_raw) {
            Ok(set) => {
                let path = storage::flock_camera_false_positive_ignore_path();
                match storage::sd_io_handle().map(|sd_io| {
                    storage::save_mac_ignore_list(
                        sd_io,
                        &path,
                        storage::FLOCK_IGNORE_FILE_HEADER,
                        &set,
                    )
                }) {
                    Some(Ok(())) => {
                        if let Ok(mut w) = detection_ex_s.write() {
                            *w = set;
                        }
                        web_snapshots_bump_forms_rev();
                        "/detections?msg=ok".to_string()
                    }
                    Some(Err(e)) => format!(
                        "/detections?msg=err&detail={}",
                        query_param_escape(&format!("Write failed: {e}"))
                    ),
                    None => "/detections?msg=err&detail=SD%20I%2FO%20not%20ready".to_string(),
                }
            }
            Err(e) => format!("/detections?msg=err&detail={}", query_param_escape(&e)),
        }
    };

    see_other(req, &location)
}

pub(crate) fn register_setup_routes(
    http: &mut EspHttpServer<'static>,
    _sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    exclude_ssids_manual: Arc<RwLock<HashSet<String>>>,
    detection_exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    ssid_watchlist: Arc<RwLock<HashSet<String>>>,
    denylist_dirty: Arc<AtomicBool>,
    watchlist_dirty: Arc<AtomicBool>,
    nvs_defer: NvsDeferQueue,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
) -> anyhow::Result<()> {
    let nvs_defer_save = nvs_defer.clone();
    http.fn_handler(
        "/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);

            let sta_ssid = form.get("sta_ssid").cloned().unwrap_or_default();
            let sta_pass = form.get("sta_pass").cloned().unwrap_or_default();
            let ap_ssid = form.get("ap_ssid").cloned().unwrap_or_default();
            let ap_pass = form.get("ap_pass").cloned().unwrap_or_default();
            let scan_ble = form
                .get("scan_ble")
                .map(|v| v == "1" || v == "on")
                .unwrap_or(false);
            let upload_on_boot = form
                .get("upload_on_boot")
                .map(|v| v == "1" || v == "on")
                .unwrap_or(true);

            nvs_defer_save
                .submit_and_wait(
                    NvsDeferOp::SaveStaSetup {
                        sta_ssid,
                        sta_pass,
                        ap_ssid,
                        ap_pass,
                        scan_ble,
                        upload_on_boot,
                    },
                    Duration::from_millis(NVS_DEFER_WAIT_MS),
                )
                .map_err(|e| anyhow!("{e}"))?;

            req.into_ok_response()?
                .write_all(b"<html><body><h1>Saved.</h1><p>Rebooting...</p></body></html>")?;
            Ok(())
        },
    )?;

    let ex_wfs = Arc::clone(&exclude_manual);
    let ex_ssids_wfs = Arc::clone(&exclude_ssids_manual);
    let dirty_wfs = Arc::clone(&denylist_dirty);
    http.fn_handler(
        "/wigle-filters/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let macs_raw = form.get("macs").cloned().unwrap_or_default();
            let ssids_raw = form.get("ssids").cloned().unwrap_or_default();

            let location = if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
                "/wigle-filters?msg=err&detail=SD%20card%20must%20be%20mounted".to_string()
            } else {
                let mac_result = storage::parse_mac_ignore_file_text(&macs_raw);
                let ssid_result = storage::parse_ssid_ignore_file_text(&ssids_raw);
                match (mac_result, ssid_result) {
                    (Ok(mac_set), Ok(ssid_set)) => {
                        let mac_path = storage::wigle_manual_bssid_ignore_path();
                        let ssid_path = storage::wigle_manual_ssid_ignore_path();
                        let mac_write = storage::sd_io_handle().map(|sd_io| {
                            storage::save_mac_ignore_list(
                                sd_io,
                                &mac_path,
                                storage::WIGLE_MANUAL_IGNORE_FILE_HEADER,
                                &mac_set,
                            )
                        });
                        let ssid_write = storage::sd_io_handle().map(|sd_io| {
                            storage::save_ssid_ignore_list(
                                sd_io,
                                &ssid_path,
                                storage::WIGLE_MANUAL_SSID_IGNORE_FILE_HEADER,
                                &ssid_set,
                            )
                        });
                        match (mac_write, ssid_write) {
                            (Some(Ok(())), Some(Ok(()))) => {
                                if let Ok(mut w) = ex_wfs.write() {
                                    *w = mac_set;
                                }
                                if let Ok(mut w) = ex_ssids_wfs.write() {
                                    *w = ssid_set;
                                }
                                dirty_wfs.store(true, Ordering::Release);
                                web_snapshots_bump_forms_rev();
                                "/wigle-filters?msg=ok".to_string()
                            }
                            (Some(Err(e)), _) | (_, Some(Err(e))) => format!(
                                "/wigle-filters?msg=err&detail={}",
                                query_param_escape(&format!("Write failed: {e}"))
                            ),
                            (None, _) | (_, None) => {
                                "/wigle-filters?msg=err&detail=sd-io%20handle%20not%20registered"
                                    .to_string()
                            }
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        format!("/wigle-filters?msg=err&detail={}", query_param_escape(&e))
                    }
                }
            };

            see_other(req, &location)
        },
    )?;

    let detection_ex_s = Arc::clone(&detection_exclude_manual);
    http.fn_handler(
        "/detection-ignore/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_ignore_save(req, &detection_ex_s)
        },
    )?;

    // Legacy alias: same handler as `/detection-ignore/save`.
    let detection_ex_s_alias = Arc::clone(&detection_exclude_manual);
    http.fn_handler(
        "/flock-ignore/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            handle_detection_ignore_save(req, &detection_ex_s_alias)
        },
    )?;

    let ssid_watch_s = Arc::clone(&ssid_watchlist);
    let watch_dirty_s = Arc::clone(&watchlist_dirty);
    http.fn_handler(
        "/ssid-watch/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let ssids_raw = form.get("ssids").cloned().unwrap_or_default();

            let location = if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
                "/detections?msg=err&detail=SD%20card%20must%20be%20mounted".to_string()
            } else {
                match storage::parse_ssid_watchlist_file_text(&ssids_raw) {
                    Ok(set) => {
                        let path = storage::ssid_watchlist_path();
                        match storage::sd_io_handle()
                            .map(|sd_io| storage::save_ssid_watchlist(sd_io, &path, &set))
                        {
                            Some(Ok(())) => {
                                if let Ok(mut w) = ssid_watch_s.write() {
                                    *w = set;
                                }
                                watch_dirty_s.store(true, Ordering::Release);
                                web_snapshots_bump_forms_rev();
                                "/detections?msg=ok".to_string()
                            }
                            Some(Err(e)) => format!(
                                "/detections?msg=err&detail={}",
                                query_param_escape(&format!("Write failed: {e}"))
                            ),
                            None => {
                                "/detections?msg=err&detail=SD%20I%2FO%20not%20ready".to_string()
                            }
                        }
                    }
                    Err(e) => format!("/detections?msg=err&detail={}", query_param_escape(&e)),
                }
            };

            see_other(req, &location)
        },
    )?;

    let nvs_defer_hzs = nvs_defer;
    let gps_hzs = Arc::clone(&gps_fix);
    http.fn_handler("/home-zone/save", Method::Post, move |mut req| -> Result<(), anyhow::Error> {
        let raw = read_post_form(&mut req, 1024);
        let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
        let action = form.get("action").map(String::as_str).unwrap_or("save");
        let wait = Duration::from_millis(NVS_DEFER_WAIT_MS);

        let location = match action {
            "clear" => redirect_home_geo_result(
                nvs_defer_hzs.submit_and_wait(NvsDeferOp::HomeGeoClear, wait),
            ),
            "from_gps" => {
                let rad_s = form.get("radius_m").cloned().unwrap_or_default();
                let radius_parse = if rad_s.trim().is_empty() {
                    Ok(config::DEFAULT_HOME_GEO_RADIUS_M)
                } else {
                    rad_s
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| "invalid radius (meters)".to_string())
                };
                match radius_parse {
                    Err(msg) => format!(
                        "/setup?msg=err&detail={}",
                        query_param_escape(&msg)
                    ),
                    Ok(radius_m) => {
                        if !(config::MIN_HOME_GEO_RADIUS_M..=config::MAX_HOME_GEO_RADIUS_M)
                            .contains(&radius_m)
                        {
                            format!(
                                "/setup?msg=err&detail=radius%20must%20be%20{}..%3D{}%20meters",
                                config::MIN_HOME_GEO_RADIUS_M,
                                config::MAX_HOME_GEO_RADIUS_M
                            )
                        } else {
                            match gps_hzs.lock().ok().and_then(|g| g.clone()) {
                                None => "/setup?msg=err&detail=No%20GPS%20fix%20yet".to_string(),
                                Some(f) if !config::home_geo_fix_meets_scan_gate(&f) => {
                                    "/setup?msg=err&detail=Fix%20quality%20or%20HDOP%20does%20not%20meet%20wardriving%20gate"
                                        .to_string()
                                }
                                Some(f) => redirect_home_geo_result(nvs_defer_hzs.submit_and_wait(
                                    NvsDeferOp::HomeGeoSave {
                                        lat: f.lat,
                                        lon: f.lon,
                                        radius_m,
                                    },
                                    wait,
                                )),
                            }
                        }
                    }
                }
            }
            _ => {
                let lat = form.get("lat").cloned().unwrap_or_default();
                let lon = form.get("lon").cloned().unwrap_or_default();
                let rad = form.get("radius_m").cloned().unwrap_or_default();
                match config::parse_home_geo_form(&lat, &lon, &rad) {
                    Err(e) => format!(
                        "/setup?msg=err&detail={}",
                        query_param_escape(&e)
                    ),
                    Ok((la, lo, radius_m)) => redirect_home_geo_result(
                        nvs_defer_hzs.submit_and_wait(
                            NvsDeferOp::HomeGeoSave {
                                lat: la,
                                lon: lo,
                                radius_m,
                            },
                            wait,
                        ),
                    ),
                }
            }
        };

        see_other(req, &location)
    })?;

    Ok(())
}
