//! HTTP route registration for the control-node web UI.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};

use crate::control::NvsDeferQueue;
use crate::gps;
use crate::storage;

use super::routes_api;
use super::routes_setup;
use super::routes_static;
use super::routes_uploads;
use super::snapshots::WebSnapshotState;

/// Shared state captured by HTTP handlers at registration time.
pub(crate) struct WebRouteDeps {
    pub(crate) nvs: Arc<EspNvsPartition<NvsDefault>>,
    pub(crate) exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub(crate) exclude_ssids_manual: Arc<RwLock<HashSet<String>>>,
    pub(crate) detection_exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub(crate) ssid_watchlist: Arc<RwLock<HashSet<String>>>,
    pub(crate) denylist_dirty: Arc<AtomicBool>,
    pub(crate) watchlist_dirty: Arc<AtomicBool>,
    pub(crate) detection_masks_dirty: Arc<AtomicBool>,
    pub(crate) nvs_defer: NvsDeferQueue,
    pub(crate) gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    pub(crate) sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    pub(crate) web_snapshots: Arc<RwLock<WebSnapshotState>>,
    /// Control boot session id echoed to sniffers on ESP-NOW session changes.
    pub(crate) control_boot_id: u32,
    pub(crate) cmd_seq: Arc<AtomicU32>,
    pub(crate) wigle_home: Arc<Mutex<Option<[u8; 6]>>>,
    pub(crate) own_management_ap_bssid: Option<[u8; 6]>,
    pub(crate) sd_io: Arc<storage::SdIoHandle>,
}

pub(crate) fn register_routes(
    http: &mut EspHttpServer<'static>,
    deps: WebRouteDeps,
) -> Result<(), anyhow::Error> {
    routes_static::register_static_pages(http, Arc::clone(&deps.web_snapshots))?;
    routes_setup::register_setup_routes(
        http,
        Arc::clone(&deps.sniffers),
        Arc::clone(&deps.exclude_manual),
        Arc::clone(&deps.exclude_ssids_manual),
        Arc::clone(&deps.detection_exclude_manual),
        Arc::clone(&deps.ssid_watchlist),
        Arc::clone(&deps.denylist_dirty),
        Arc::clone(&deps.watchlist_dirty),
        deps.nvs_defer.clone(),
        Arc::clone(&deps.gps_fix),
    )?;
    routes_uploads::register_upload_routes(http, deps.nvs_defer.clone(), Arc::clone(&deps.nvs))?;
    routes_api::register_api_routes(http, &deps)?;
    Ok(())
}
