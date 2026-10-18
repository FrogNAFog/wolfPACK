//! Control boot sequencing helpers (SD post-mount, alert hub, STA upload window).

use std::collections::HashSet;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::Result;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::{info, warn};

use super::boot_setup;
use super::exclusion::apply_exclusion_lists_from_boot_snapshot;
use crate::config;
use crate::storage;
use crate::wifi_scan;
use wardriver_shared::wifi_tx_power;
use wardriver_stats as stats;

/// WiGLE / LPR-camera exclusion state created before SD mount.
pub(crate) struct ExclusionState {
    pub wigle_exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    pub wigle_exclude_ssids_manual: Arc<RwLock<HashSet<String>>>,
    pub detection_exclude_manual: Arc<RwLock<HashSet<[u8; 6]>>>,
    /// Beacon SSID watchlist (alert-only; `/detections`).
    pub ssid_watchlist: Arc<RwLock<HashSet<String>>>,
    pub wigle_home_bssid: Arc<Mutex<Option<[u8; 6]>>>,
    pub denylist_dirty: Arc<AtomicBool>,
    /// Set when SSID watchlist changes; main loop rebroadcasts `SsidWatchlist` frames.
    pub watchlist_dirty: Arc<AtomicBool>,
    /// Set when `flock_disable_*_mask` changes; main loop rebroadcasts via `ControlCmd` (Ping).
    pub detection_masks_dirty: Arc<AtomicBool>,
    pub home_geo: Arc<Mutex<config::HomeGeoConfig>>,
}

impl ExclusionState {
    pub(crate) fn new(nvs: &EspNvsPartition<NvsDefault>) -> Self {
        Self {
            wigle_exclude_manual: Arc::new(RwLock::new(HashSet::new())),
            wigle_exclude_ssids_manual: Arc::new(RwLock::new(HashSet::new())),
            detection_exclude_manual: Arc::new(RwLock::new(HashSet::new())),
            ssid_watchlist: Arc::new(RwLock::new(HashSet::new())),
            wigle_home_bssid: Arc::new(Mutex::new(
                config::wigle_home_bssid_load(nvs).unwrap_or(None),
            )),
            denylist_dirty: Arc::new(AtomicBool::new(false)),
            watchlist_dirty: Arc::new(AtomicBool::new(false)),
            detection_masks_dirty: Arc::new(AtomicBool::new(false)),
            home_geo: Arc::new(Mutex::new(config::home_geo_load(nvs).unwrap_or_default())),
        }
    }
}

/// SD mounted successfully: apply worker-collected boot snapshot in RAM (no main-thread VFS).
pub(crate) fn on_sd_mount_ok(
    nvs: &EspNvsPartition<NvsDefault>,
    exclusion: &ExclusionState,
    _sd_io: &storage::SdIoHandle,
) {
    let snap = storage::take_boot_sd_snapshot().unwrap_or_default();
    let prefs_boot = crate::preferences::apply_from_boot_snapshot(snap.preferences.as_deref());
    crate::preferences::apply_to_stats(&prefs_boot);
    crate::channel_plan::apply_from_boot_snapshot(snap.channel_plan.as_deref());
    if let Err(e) = apply_exclusion_lists_from_boot_snapshot(
        nvs,
        &snap,
        &exclusion.wigle_exclude_manual,
        &exclusion.wigle_exclude_ssids_manual,
        &exclusion.detection_exclude_manual,
        &exclusion.ssid_watchlist,
    ) {
        warn!("control: exclusion lists from SD snapshot: {e:#}");
    }
}

/// SD mount failed: NVS fallback for WiGLE manual exclusions; alert operator on XIAO.
pub(crate) fn on_sd_mount_fail(
    nvs: &EspNvsPartition<NvsDefault>,
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
    exclusion: &ExclusionState,
    err: &anyhow::Error,
) {
    #[cfg(feature = "board-marauder")]
    warn!(
        "wolfpack/sd: mount FAILED at {} (check card, format FAT32, wiring vs Marauder C5 pin map): {err:#}",
        storage::MOUNT_POINT
    );
    #[cfg(feature = "board-xiao")]
    {
        let bp = boot_setup::board_profile();
        warn!(
            "wolfpack/sd: mount FAILED at {} (check card, format FAT32, Seeed expansion base SD: D8/D9/D10 + {}=GPIO{}): {err:#}",
            storage::MOUNT_POINT,
            bp.sd_cs_label,
            bp.sd_cs
        );
    }
    alert_hub.trigger(crate::alert_feedback::AlertKind::SdFail, None);
    stats::set_sd_mounted(false);
    if let Ok(s) = config::wigle_exclude_macs_load(nvs) {
        if let Ok(mut w) = exclusion.wigle_exclude_manual.write() {
            *w = s;
        }
    }
    warn!(
        "control: SD absent — WiGLE manual exclusions loaded from NVS (if any); LPR-camera ignore list unavailable until SD is mounted"
    );
}

/// Post-GPS boot OLED / alert trigger (defers the post-boot GPS status line when GPS+SD both OK).
pub(crate) fn trigger_post_boot_alerts(
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
    gps_has_fix: bool,
) {
    let post_boot_gps = if gps_has_fix && stats::sd_mounted() {
        None
    } else {
        Some(crate::alert_feedback::PostBootGpsOled::Deferred {
            gps_ok: gps_has_fix,
        })
    };
    alert_hub.trigger_boot(post_boot_gps);
}

/// Bring up the management SoftAP for dashboard / provisioning (deferred until after boot upload when possible).
pub(crate) fn ensure_management_softap(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
) -> Result<()> {
    boot_setup::start_control_ap(wifi, cfg)?;
    crate::provisioning::wait_ap_netif_ready(wifi, Duration::from_secs(20))
        .map_err(|e| anyhow::anyhow!("control: SoftAP netif not ready: {e}"))?;
    if let Err(e) = boot_setup::configure_management_softap_netif(wifi) {
        warn!("control: SoftAP IP/DHCP config: {e:#}");
    }
    wifi_tx_power::apply_default_cap_once();
    Ok(())
}

/// STA connect, optional home BSSID learn, boot upload, SoftAP channel restore.
///
/// Returns `true` when this function restored the management SoftAP (upload window ran or STA failed).
/// Returns `false` when the caller must bring up SoftAP (unprovisioned, or upload window skipped).
pub(crate) fn boot_sta_upload_window(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
    nvs: &EspNvsPartition<NvsDefault>,
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
    exclusion: &ExclusionState,
    sd_io: Arc<storage::SdIoHandle>,
) -> bool {
    if !cfg.provisioning_complete() {
        warn!(
            "wolfpack/boot: home WiFi SSID not saved; STA connect skipped — use /setup on SoftAP"
        );
        return false;
    }

    let paths = storage::Paths::new();
    if !boot_setup::boot_upload_configured(cfg) || stats::upload_pending_csv_count() == 0 {
        info!("wolfpack/boot: skipping STA upload window (upload disabled or no pending files)");
        return false;
    }

    info!("wolfpack/boot: STA connect (boot upload)");
    match boot_setup::connect_sta_only(wifi, cfg) {
        Ok(()) => {
            if let Err(e) =
                boot_setup::learn_home_wigle_bssid(wifi, cfg, nvs, &exclusion.wigle_home_bssid)
            {
                warn!("wolfpack/boot: learn wg_home_bssid: {e:#}");
            }
            boot_setup::control_upload_pending(
                Arc::clone(alert_hub),
                cfg,
                &paths,
                Arc::clone(&sd_io),
                "wolfpack/boot",
                true,
            );
            boot_setup::restore_control_softap_home_channel(wifi, cfg);
            true
        }
        Err(e) => {
            warn!("wolfpack/boot: STA connect failed: {e:#}");
            boot_setup::restore_control_softap_home_channel(wifi, cfg);
            true
        }
    }
}

/// Whether the current GPS snapshot is usable for wardrive CSV location fields.
pub(crate) fn gps_has_usable_fix(gps_fix: &Arc<Mutex<Option<crate::gps::Fix>>>) -> bool {
    gps_fix
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|f| wifi_scan::is_fix_usable_for_location(&f))
        .unwrap_or(false)
}
