//! Prebuilt HTTP response bodies for GET handlers (control loop builds; httpd copies only).
//!
//! JSON is built **outside** the [`RwLock`] write guard; the loop applies updates with
//! [`try_write`](std::sync::RwLock::try_write) so a slow httpd reader cannot wedge the main loop.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use embedded_io::ErrorType;
use embedded_io::Write as _;
use esp_idf_svc::http::server::{Connection, Request};
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};

use crate::config::HomeGeoConfig;
use crate::gps;
use crate::storage;
use wardriver_stats as stats;

use super::api_stats;
use super::channel_plan_api;
use super::format_bssid;
use super::json_escape;

pub const CT_CSS: &str = "text/css; charset=utf-8";
pub const CT_JSON: &str = "application/json";
pub const CT_HTML: &str = "text/html; charset=utf-8";

/// Immutable body + MIME type served by GET handlers.
#[derive(Clone)]
pub struct WebResponseSnapshot {
    pub body: Arc<[u8]>,
    pub content_type: &'static str,
}

/// Latest prebuilt GET payloads (CSS fixed at boot; JSON refreshed in the control loop).
pub struct WebSnapshots {
    pub theme_css: WebResponseSnapshot,
    pub simple_form_css: WebResponseSnapshot,
    pub stats: WebResponseSnapshot,
    pub channel_plan: WebResponseSnapshot,
    pub ui_forms: WebResponseSnapshot,
}

/// Control-loop state: snapshots plus rebuild bookkeeping.
pub struct WebSnapshotState {
    pub snapshots: WebSnapshots,
    pub last_stats_build_ms: u64,
    pub last_channel_plan_rev: u32,
    pub last_forms_rev: u32,
    pub last_nvs_display_rev: u32,
}

/// Live inputs for snapshot rebuild (Arc refs — no NVS reads on httpd).
pub struct WebSnapshotInputs<'a> {
    pub nvs: &'a EspNvsPartition<NvsDefault>,
    pub home_geo: &'a Arc<Mutex<HomeGeoConfig>>,
    pub gps_fix: &'a Arc<Mutex<Option<gps::Fix>>>,
    pub exclude_manual: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    pub exclude_ssids_manual: &'a Arc<RwLock<HashSet<String>>>,
    pub detection_exclude_manual: &'a Arc<RwLock<HashSet<[u8; 6]>>>,
    pub ssid_watchlist: &'a Arc<RwLock<HashSet<String>>>,
}

static FORMS_REV: AtomicU32 = AtomicU32::new(0);
static NVS_DISPLAY_REV: AtomicU32 = AtomicU32::new(0);
static WIGLE_API_NAME_DISPLAY: Mutex<String> = Mutex::new(String::new());

/// True when a dashboard page polled `/api/stats` within the idle window.
fn dashboard_recently_polled(now_ms: u64) -> bool {
    let last = stats::last_dashboard_stats_fetch_ms();
    last != 0 && now_ms.saturating_sub(last) < stats::DASHBOARD_STATS_IDLE_MS
}

/// Whether this main-loop tick should run `build_api_stats_json`.
///
/// Wardrive with no recent polls: slower periodic rebuild ([`stats::stats_rebuild_interval_idle_ms`]).
/// httpd sets [`stats::request_stats_rebuild`] on idle → active poll transitions.
fn stats_rebuild_due(state: &WebSnapshotState, now_ms: u64, nvs_rev: u32) -> bool {
    // Boot placeholder is `{}` until the first successful build.
    if state.last_stats_build_ms == 0 {
        return true;
    }
    // Idle → active wardrive: poll arrived on httpd; rebuild here (~50 ms), not in the handler.
    if stats::take_stats_rebuild_requested() {
        return true;
    }
    let nvs_changed = nvs_rev != state.last_nvs_display_rev;
    let active_interval_due =
        now_ms.saturating_sub(state.last_stats_build_ms) >= stats::stats_rebuild_interval_ms();
    if !stats::scan_allowed() {
        // Fleet paused: keep 1 s refresh for an operator still on the SoftAP UI.
        return active_interval_due || nvs_changed;
    }
    if dashboard_recently_polled(now_ms) {
        // Wardriving with a live dashboard tab.
        return active_interval_due || nvs_changed;
    }
    // Wardriving, no recent `/api/stats` poll: slower cadence (still periodic).
    let idle_interval_due =
        now_ms.saturating_sub(state.last_stats_build_ms) >= stats::stats_rebuild_interval_idle_ms();
    idle_interval_due || nvs_changed
}

fn bytes_snapshot(bytes: &'static [u8], content_type: &'static str) -> WebResponseSnapshot {
    WebResponseSnapshot {
        body: Arc::from(bytes),
        content_type,
    }
}

fn vec_to_snapshot(v: Vec<u8>, content_type: &'static str) -> WebResponseSnapshot {
    WebResponseSnapshot {
        body: Arc::from(v),
        content_type,
    }
}

fn empty_json_snapshot() -> WebResponseSnapshot {
    vec_to_snapshot(b"{}".to_vec(), CT_JSON)
}

/// Boot-time CSS from flash; JSON placeholders until the first control-loop tick.
pub fn web_snapshots_new() -> WebSnapshotState {
    WebSnapshotState {
        snapshots: WebSnapshots {
            theme_css: bytes_snapshot(
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/web_theme.css")),
                CT_CSS,
            ),
            simple_form_css: bytes_snapshot(
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/web_simple_form.css"
                )),
                CT_CSS,
            ),
            stats: empty_json_snapshot(),
            channel_plan: empty_json_snapshot(),
            ui_forms: empty_json_snapshot(),
        },
        last_stats_build_ms: 0,
        // Force first-tick rebuild (live rev starts at 0).
        last_channel_plan_rev: u32::MAX,
        last_forms_rev: u32::MAX,
        last_nvs_display_rev: u32::MAX,
    }
}

pub fn web_snapshots_init() -> Arc<RwLock<WebSnapshotState>> {
    Arc::new(RwLock::new(web_snapshots_new()))
}

/// Current ui-forms revision (embedded in `/api/ui-forms` and `/api/stats`).
pub fn forms_rev() -> u32 {
    FORMS_REV.load(Ordering::Acquire)
}

/// Call after POST saves that change wigle-filters or LPR-camera-ignore form data.
pub fn web_snapshots_bump_forms_rev() {
    FORMS_REV.fetch_add(1, Ordering::Release);
}

/// Update displayed WiGLE API name (from uploads save); bumps [`NVS_DISPLAY_REV`] for stats rebuild.
pub fn web_snapshots_set_wigle_api_name(s: &str) {
    if let Ok(mut g) = WIGLE_API_NAME_DISPLAY.lock() {
        *g = s.to_owned();
    }
    NVS_DISPLAY_REV.fetch_add(1, Ordering::Release);
}

fn wigle_api_name_display() -> String {
    WIGLE_API_NAME_DISPLAY
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Merge former `/api/form/wigle-filters` + `/api/form/detection-ignore` into one JSON blob.
pub(crate) fn build_ui_forms_json(inputs: &WebSnapshotInputs<'_>) -> String {
    let home_note = match crate::config::wigle_home_bssid_load(inputs.nvs) {
        Ok(Some(m)) => format_bssid(&m),
        Ok(None) => {
            "none yet (learned after STA connect on boot when WiGLE API credentials are set)"
                .to_string()
        }
        Err(_) => "(read error)".to_string(),
    };
    let wigle_sd_path = format!(
        "{}/{}",
        storage::MOUNT_POINT.trim_end_matches('/'),
        storage::RELPATH_WIGLE_MANUAL_BSSID_IGNORE
    );
    let wigle_ssid_sd_path = format!(
        "{}/{}",
        storage::MOUNT_POINT.trim_end_matches('/'),
        storage::RELPATH_WIGLE_MANUAL_SSID_IGNORE
    );
    let detection_sd_path = format!(
        "{}/{}",
        storage::MOUNT_POINT.trim_end_matches('/'),
        storage::RELPATH_FLOCK_CAMERA_FALSE_POSITIVE_IGNORE
    );
    let ssid_watch_sd_path = format!(
        "{}/{}",
        storage::MOUNT_POINT.trim_end_matches('/'),
        storage::RELPATH_SSID_WATCHLIST
    );

    let mut json = String::from("{\"forms_rev\":");
    json.push_str(&forms_rev().to_string());
    json.push_str(",\"wigle\":{\"home_note\":\"");
    json.push_str(&json_escape(&home_note));
    json.push_str("\",\"sd_path\":\"");
    json.push_str(&json_escape(&wigle_sd_path));
    json.push_str("\",\"macs\":[");
    if let Ok(set) = inputs.exclude_manual.read() {
        let mut macs: Vec<_> = set.iter().copied().collect();
        macs.sort();
        for (i, m) in macs.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push('"');
            json.push_str(&format_bssid(m));
            json.push('"');
        }
    }
    json.push_str("],\"ssids\":[");
    if let Ok(set) = inputs.exclude_ssids_manual.read() {
        let mut ssids: Vec<_> = set.iter().cloned().collect();
        ssids.sort();
        for (i, s) in ssids.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push('"');
            json.push_str(&json_escape(s));
            json.push('"');
        }
    }
    json.push_str("],\"ssid_sd_path\":\"");
    json.push_str(&json_escape(&wigle_ssid_sd_path));
    json.push_str("\"},\"detection\":{\"sd_path\":\"");
    json.push_str(&json_escape(&detection_sd_path));
    json.push_str("\",\"macs\":[");
    if let Ok(set) = inputs.detection_exclude_manual.read() {
        let mut macs: Vec<_> = set.iter().copied().collect();
        macs.sort();
        for (i, m) in macs.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push('"');
            json.push_str(&format_bssid(m));
            json.push('"');
        }
    }
    json.push_str("]},\"ssid_watch\":{\"sd_path\":\"");
    json.push_str(&json_escape(&ssid_watch_sd_path));
    json.push_str("\",\"ssids\":[");
    if let Ok(set) = inputs.ssid_watchlist.read() {
        let mut ssids: Vec<_> = set.iter().cloned().collect();
        ssids.sort();
        for (i, s) in ssids.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push('"');
            json.push_str(&json_escape(s));
            json.push('"');
        }
    }
    json.push_str("]}}");
    json
}

fn record_rebuild_us(t0_ms: u64) {
    let elapsed_ms = stats::now_ms().saturating_sub(t0_ms);
    stats::WEB_SNAPSHOT_REBUILD_US.fetch_add(
        elapsed_ms.saturating_mul(1_000).min(u32::MAX as u64) as u32,
        Ordering::Relaxed,
    );
}

/// Bodies to swap under a brief write lock (built without holding [`WebSnapshotState`]).
pub struct SnapshotRebuildPlan {
    pub stats: Option<WebResponseSnapshot>,
    pub stats_build_ms: Option<u64>,
    pub stats_nvs_rev: Option<u32>,
    pub channel_plan: Option<WebResponseSnapshot>,
    pub channel_plan_rev: Option<u32>,
    pub ui_forms: Option<WebResponseSnapshot>,
    pub ui_forms_rev: Option<u32>,
}

/// Decide what to rebuild and construct JSON off-lock (may take milliseconds).
pub fn plan_snapshot_rebuilds(
    state: &WebSnapshotState,
    inputs: &WebSnapshotInputs<'_>,
    now_ms: u64,
) -> SnapshotRebuildPlan {
    let mut plan = SnapshotRebuildPlan {
        stats: None,
        stats_build_ms: None,
        stats_nvs_rev: None,
        channel_plan: None,
        channel_plan_rev: None,
        ui_forms: None,
        ui_forms_rev: None,
    };

    let nvs_rev = NVS_DISPLAY_REV.load(Ordering::Acquire);
    if stats_rebuild_due(state, now_ms, nvs_rev) {
        let wigle_name = wigle_api_name_display();
        let json =
            api_stats::build_api_stats_json(wigle_name.as_str(), inputs.home_geo, inputs.gps_fix);
        plan.stats = Some(vec_to_snapshot(json.into_bytes(), CT_JSON));
        plan.stats_build_ms = Some(now_ms);
        plan.stats_nvs_rev = Some(nvs_rev);
    }

    let cp_rev = stats::CHANNEL_PLAN_REVISION.load(Ordering::Acquire);
    if cp_rev != state.last_channel_plan_rev {
        let json = channel_plan_api::channel_plan_response_json(false, None);
        plan.channel_plan = Some(vec_to_snapshot(json.into_bytes(), CT_JSON));
        plan.channel_plan_rev = Some(cp_rev);
    }

    let forms = forms_rev();
    if forms != state.last_forms_rev {
        let json = build_ui_forms_json(inputs);
        plan.ui_forms = Some(vec_to_snapshot(json.into_bytes(), CT_JSON));
        plan.ui_forms_rev = Some(forms);
    }

    plan
}

/// Apply a pre-built plan under the write lock. Returns true if any snapshot was updated.
pub fn apply_snapshot_rebuild_plan(
    state: &mut WebSnapshotState,
    plan: SnapshotRebuildPlan,
) -> bool {
    let mut rebuilt = false;
    if let Some(snap) = plan.stats {
        state.snapshots.stats = snap;
        if let Some(ms) = plan.stats_build_ms {
            state.last_stats_build_ms = ms;
            stats::note_control_snapshot_build_ms(ms);
        }
        if let Some(rev) = plan.stats_nvs_rev {
            state.last_nvs_display_rev = rev;
        }
        rebuilt = true;
    }
    if let Some(snap) = plan.channel_plan {
        state.snapshots.channel_plan = snap;
        if let Some(rev) = plan.channel_plan_rev {
            state.last_channel_plan_rev = rev;
        }
        rebuilt = true;
    }
    if let Some(snap) = plan.ui_forms {
        state.snapshots.ui_forms = snap;
        if let Some(rev) = plan.ui_forms_rev {
            state.last_forms_rev = rev;
        }
        rebuilt = true;
    }
    rebuilt
}

/// Build off-lock, then [`try_write`](std::sync::RwLock::try_write) + apply. Skips on contention.
pub fn web_snapshots_tick_try(
    snapshots: &std::sync::RwLock<WebSnapshotState>,
    inputs: &WebSnapshotInputs<'_>,
    now_ms: u64,
) {
    let t0 = stats::now_ms();
    // Snapshot bookkeeping for planning only — stale rev by one tick is acceptable.
    let plan_book = snapshots
        .read()
        .map(|g| {
            (
                g.last_stats_build_ms,
                g.last_channel_plan_rev,
                g.last_forms_rev,
                g.last_nvs_display_rev,
            )
        })
        .unwrap_or((0, u32::MAX, u32::MAX, u32::MAX));
    let plan_state = WebSnapshotState {
        snapshots: WebSnapshots {
            theme_css: empty_json_snapshot(),
            simple_form_css: empty_json_snapshot(),
            stats: empty_json_snapshot(),
            channel_plan: empty_json_snapshot(),
            ui_forms: empty_json_snapshot(),
        },
        last_stats_build_ms: plan_book.0,
        last_channel_plan_rev: plan_book.1,
        last_forms_rev: plan_book.2,
        last_nvs_display_rev: plan_book.3,
    };
    let plan = plan_snapshot_rebuilds(&plan_state, inputs, now_ms);
    if !plan.stats.is_some() && !plan.channel_plan.is_some() && !plan.ui_forms.is_some() {
        return;
    }

    match snapshots.try_write() {
        Ok(mut state) => {
            if apply_snapshot_rebuild_plan(&mut state, plan) {
                record_rebuild_us(t0);
            }
        }
        Err(std::sync::TryLockError::WouldBlock) => {
            stats::note_control_snapshot_write_skipped();
        }
        Err(std::sync::TryLockError::Poisoned(_)) => {
            stats::note_control_snapshot_write_skipped();
        }
    }
}

/// GET handler helper: 200 + Content-Type + body copy (no allocation).
pub fn write_snapshot<C: ErrorType + Connection>(
    req: Request<C>,
    snap: &WebResponseSnapshot,
) -> Result<(), anyhow::Error> {
    let mut resp = req
        .into_response(200, Some("OK"), &[("Content-Type", snap.content_type)])
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    resp.write_all(&snap.body)
        .map_err(|e| anyhow::anyhow!("{e:?}"))
}

/// Static HTML from `include_str!` with explicit charset.
pub fn write_flash_html<C: ErrorType + Connection>(
    req: Request<C>,
    html: &str,
) -> Result<(), anyhow::Error> {
    let mut resp = req
        .into_response(200, Some("OK"), &[("Content-Type", CT_HTML)])
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    resp.write_all(html.as_bytes())
        .map_err(|e| anyhow::anyhow!("{e:?}"))
}
