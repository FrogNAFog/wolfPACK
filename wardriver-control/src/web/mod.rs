//! Always-on web UI (served on the device SoftAP) showing live statistics.
//!
//! **Control-node only:** `main.rs` includes this module only with `feature = "control"`.
//! Sniffer firmware never compiles or links this file.
//!
//! **httpd copy-only contract:** GET handlers [`write_all`] from flash (`include_str!` HTML) or
//! prebuilt [`snapshots::WebResponseSnapshot`] bodies (CSS/JSON built in the control loop via
//! [`snapshots::web_snapshots_tick`], not on the httpd worker stack).
//!
//! Endpoints:
//! - `GET /`           : fleet dashboard HTML (mobile-first; detection alert banner)
//! - `GET /sniffers`   : fleet role assignment UI (`POST /sniffers/role`)
//! - `GET /diagnostics`: deep `/api/stats` tables + raw JSON
//! - `GET /detections` : LPR-camera detector masks + MAC ignore list
//! - `GET /capture` : WiFi probe PCAP collector + job catalog
//! - `GET /recon` : 303 redirect to `/capture` (legacy path)
//! - `GET /api/capture/catalog` : JSON list of available capture jobs (legacy `/api/recon/catalog`)
//! - `POST /api/capture/start` : `job=wifi_probe_pcap` queues a 60s fleet capture (SD required; legacy `/api/recon/start`)
//! - `GET /api/stats`  : JSON including `management_ap_ssid` (SoftAP name only), `control_mem_heap_avail_pct`, `control_mem_heap_{free,total}_bytes` (internal + SPIRAM when present), `control_cpu_load_pct` (FreeRTOS run-time stats), and per-sniffer `mem_heap_{free,total,avail_pct}` and `cpu_load_pct` from ACKs; plus SD/GPS, ESP-NOW, drops, `unique_wifi_aps` / `unique_ble_devices`, unified `mac_table_*` gauges, `sniffers[]` (per-media `wifi_*` / `ble_*` frag, row, reasm, and bulk-lost fields plus legacy sum aliases, sweep metrics, …). `scan_allowed` arms sniffers; `csv_enqueue_allowed` gates WiGLE rows. `wifi_rows_written` counts rows after `sd_session_created`. Batch drops / unpack counters include WiFi and BLE (see raw JSON on `/diagnostics` for the full key set). During recon: `recon_remaining_ms`, `recon_elapsed_ms` (monotonic capture window).
//! - `POST /api/mode`  : `mode=wardrive` or `mode=management` (control only; queues mode change)
//! - `GET /setup`      : provisioning + home-zone HTML (static; form data via `/api/stats`)
//! - `POST /save`      : provisioning save + reboot (reuses provisioning handler)
//! - `GET /wigle-filters` / `POST /wigle-filters/save` : manual BSSID/SSID exclusions for WiGLE (SD-backed)
//! - `GET /api/ui-forms` : JSON for wigle-filters + detection-ignore forms (prebuilt snapshot)
//! - `POST /detection-ignore/save` (+ legacy `/flock-ignore/save`) : LPR-camera false-positive STA MAC ignores (SD-backed); list UI on `/detections`
//! - `POST /api/detection-ignore-mac` (+ legacy `/api/flock-ignore-mac`) : append one MAC to the LPR-camera ignore set (dashboard buttons)
//! - `POST /api/detection-detectors` (+ legacy `/api/flock-detectors`) : `wifi_disable` & `ble_disable` (`u32` bitmasks) plus optional `ble_active_scan` (`0`/`1`) → merge into SD `wolfPACK/config/preferences.json`
//! - `POST /api/flock` / `POST /api/axon` : **removed** — handler returns **404** + JSON (`use POST /api/detection-detectors`; Axon is BLE method **35**)
//! - `POST /home-zone/save` : GPS privacy zone (NVS via main-loop queue; form on `/setup#sec-home-zone`)
//! - `GET /uploads` / `POST /uploads/save` / `POST /uploads/test-wdgwars` / `POST /uploads/run` : API credentials + upload trigger
//! - `GET /sounds` / `POST /sounds/save` : per-event buzzer enable mask (OLED messages always on)
//! - `GET /theme.css` / `GET /simple-form.css` : shared stylesheets (linked from form pages)

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use esp_idf_svc::http::server::{Configuration as HttpCfg, EspHttpServer};
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use log::info;

use crate::control::NvsDeferQueue;
use crate::gps;

mod api_stats;
mod channel_plan_api;
mod diagnostics_snapshot;
mod pages;
mod redirect;
mod routes;
mod routes_api;
mod routes_setup;
mod routes_static;
mod routes_uploads;
pub(crate) mod snapshots;
mod stats_handler;
mod stats_json_diag;

pub use snapshots::{
    web_snapshots_bump_forms_rev, web_snapshots_init, web_snapshots_set_wigle_api_name,
};

pub(crate) use pages::{format_bssid, json_escape};

/// Read a small form-urlencoded POST body (up to `cap` bytes) from an HTTP request.
///
/// Shared across all web route files so the truncation policy (fixed 2048-byte
/// buffer, read up to `cap`, stop at EOF or `cap`, `from_utf8().unwrap_or("")`)
/// is single-sourced. `cap` is the route-specific ceiling; the on-stack buffer is
/// always 2048 bytes, so a `cap` below 2048 truncates identically to a hand-rolled
/// loop of that size.
pub(crate) fn read_post_form(req: &mut impl embedded_io::Read, cap: usize) -> String {
    let mut body = [0u8; 2048];
    let n = cap.min(body.len());
    let mut total = 0usize;
    loop {
        let r = match req.read(&mut body[total..n]) {
            Ok(x) => x,
            Err(_) => 0,
        };
        if r == 0 {
            break;
        }
        total += r;
        if total >= n {
            break;
        }
    }
    std::str::from_utf8(&body[..total])
        .unwrap_or("")
        .to_string()
}

pub(crate) static DASHBOARD_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Fleet</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_detection_banner.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_dashboard_body.html"
    )),
    r#"</body></html>"#,
);
/// Provisioning + home-zone section (values filled client-side from `/api/stats`).
pub(crate) static SETUP_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Setup</title>
    <link rel="stylesheet" href="/theme.css">
    <link rel="stylesheet" href="/simple-form.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/provisioning_body.html"
    )),
    r#"</body></html>"#,
);
/// Upload credentials page (status from `/api/stats`).
pub(crate) static UPLOADS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Uploads</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/uploads_body.html"
    )),
    r#"</body></html>"#,
);
/// Alert sound mask (values from `/api/stats`).
pub(crate) static SOUNDS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Sounds</title>
    <link rel="stylesheet" href="/theme.css">
    <link rel="stylesheet" href="/simple-form.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_alerts_body.html"
    )),
    r#"</body></html>"#,
);
pub(crate) static WIGLE_FILTERS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>WiGLE exclusions</title>
    <link rel="stylesheet" href="/theme.css">
    <link rel="stylesheet" href="/simple-form.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/wigle_filters_inner.html"
    )),
    r#"</body></html>"#,
);
pub(crate) static SNIFFERS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Sniffers</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_sniffers_body.html"
    )),
    r#"</body></html>"#,
);
pub(crate) static DIAGNOSTICS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Diagnostics</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_diagnostics_body.html"
    )),
    r#"</body></html>"#,
);
/// LPR-camera/Axon detector masks + MAC ignore — served at `/detections`.
pub(crate) static DETECTIONS_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Detections</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_detections_body.html"
    )),
    r#"</body></html>"#,
);
/// WiFi probe PCAP collector — served at `/capture`.
pub(crate) static CAPTURE_HTML: &str = concat!(
    r#"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>wolfPACK — Capture</title>
    <link rel="stylesheet" href="/theme.css">
  </head>
  <body>"#,
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_nav_fragment.html"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/assets/web_capture_body.html"
    )),
    r#"</body></html>"#,
);

pub(crate) fn start(
    nvs: Arc<EspNvsPartition<NvsDefault>>,
    exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    exclude_ssids_manual: Arc<RwLock<HashSet<String>>>,
    detection_exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    ssid_watchlist: Arc<RwLock<HashSet<String>>>,
    denylist_dirty: Arc<AtomicBool>,
    watchlist_dirty: Arc<AtomicBool>,
    detection_masks_dirty: Arc<AtomicBool>,
    nvs_defer: NvsDeferQueue,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    web_snapshots: Arc<RwLock<snapshots::WebSnapshotState>>,
    control_boot_id: u32,
    cmd_seq: Arc<AtomicU32>,
    wigle_home: Arc<Mutex<Option<[u8; 6]>>>,
    own_management_ap_bssid: Option<[u8; 6]>,
    sd_io: Arc<crate::storage::SdIoHandle>,
) -> Result<EspHttpServer<'static>> {
    let mut http = EspHttpServer::new(&HttpCfg {
        // Some handlers use multi-KiB stack buffers (form bodies + JSON formatting).
        // Keep headroom to avoid rare stack overflow resets under load.
        stack_size: 24 * 1024,
        // httpd task stack in PSRAM (requires CONFIG_SPIRAM_ALLOW_STACK_EXTERNAL_MEMORY).
        task_caps: esp_idf_sys::MALLOC_CAP_SPIRAM as u32 | esp_idf_sys::MALLOC_CAP_8BIT as u32,
        // Default is 32; we register 33+ URI handlers (channel-plan, recon, etc.).
        max_uri_handlers: 48,
        ..Default::default()
    })?;

    let deps = routes::WebRouteDeps {
        nvs,
        exclude_manual,
        exclude_ssids_manual,
        detection_exclude_manual,
        ssid_watchlist,
        denylist_dirty,
        watchlist_dirty,
        detection_masks_dirty,
        nvs_defer,
        gps_fix,
        sniffers,
        web_snapshots,
        control_boot_id,
        cmd_seq,
        wigle_home,
        own_management_ap_bssid,
        sd_io,
    };
    routes::register_routes(&mut http, deps)?;

    info!("web: started dashboard at http://192.168.71.1/");
    Ok(http)
}
