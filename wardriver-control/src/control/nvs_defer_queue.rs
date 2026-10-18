//! Deferred NVS writes from esp_http_server handlers.
//!
//! httpd runs on a PSRAM stack; NVS open/write there is fragile. Handlers enqueue ops;
//! the control main loop drains them on the internal-RAM stack (FIFO, one NVS worker).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};

use crate::config::{self, Config, HomeGeoConfig};
use crate::fleet_roster;
use crate::web::snapshots::{web_snapshots_bump_forms_rev, web_snapshots_set_wigle_api_name};
use wardriver_stats as stats;

/// Default wait for httpd handlers calling [`NvsDeferQueue::submit_and_wait`].
pub const NVS_DEFER_WAIT_MS: u64 = 2000;

static PENDING_PROVISIONING_REBOOT: AtomicBool = AtomicBool::new(false);

/// Set after a successful [`NvsDeferOp::SaveStaSetup`]; the control main loop performs
/// `esp_restart()` on the next tick (not from httpd — that path can CPU-lockup).
pub(crate) fn take_pending_provisioning_reboot() -> bool {
    PENDING_PROVISIONING_REBOOT.swap(false, Ordering::AcqRel)
}

fn request_pending_provisioning_reboot() {
    PENDING_PROVISIONING_REBOOT.store(true, Ordering::Release);
}

#[derive(Debug, Clone)]
pub enum NvsDeferOp {
    HomeGeoClear,
    HomeGeoSave {
        lat: f64,
        lon: f64,
        radius_m: u32,
    },
    FleetRosterWriteBlob(Vec<u8>),
    SaveStaSetup {
        sta_ssid: String,
        sta_pass: String,
        ap_ssid: String,
        ap_pass: String,
        scan_ble: bool,
        upload_on_boot: bool,
    },
    SaveUploadCredentials {
        wg_name: String,
        wg_token: String,
        wd_key: String,
        wd_clear: bool,
    },
    SaveAlertSound {
        mask: u8,
        volume_percent: u8,
    },
}

pub struct NvsDeferRequest {
    pub op: NvsDeferOp,
    pub reply: SyncSender<Result<(), String>>,
}

#[derive(Clone)]
pub struct NvsDeferQueue {
    tx: SyncSender<NvsDeferRequest>,
}

impl NvsDeferQueue {
    pub fn pair(capacity: usize) -> (Self, Receiver<NvsDeferRequest>) {
        let (tx, rx) = sync_channel(capacity);
        (Self { tx }, rx)
    }

    pub fn submit_and_wait(&self, op: NvsDeferOp, timeout: Duration) -> Result<(), String> {
        let (reply_tx, reply_rx) = sync_channel(1);
        let req = NvsDeferRequest {
            op,
            reply: reply_tx,
        };
        match self.tx.try_send(req) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err("NVS defer queue full".to_string());
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err("NVS defer queue disconnected".to_string());
            }
        }
        match reply_rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => Err("NVS defer request timed out".to_string()),
            Err(RecvTimeoutError::Disconnected) => Err("NVS defer reply disconnected".to_string()),
        }
    }
}

pub(crate) fn drain_nvs_defer_queue(
    rx: &Receiver<NvsDeferRequest>,
    nvs: &EspNvsPartition<NvsDefault>,
    home_geo: &Arc<Mutex<HomeGeoConfig>>,
    wigle_home: &Arc<Mutex<Option<[u8; 6]>>>,
) {
    loop {
        let req = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        };
        let result = run_nvs_defer_op(nvs, home_geo, wigle_home, req.op);
        let _ = req.reply.send(result);
    }
}

fn run_nvs_defer_op(
    nvs: &EspNvsPartition<NvsDefault>,
    home_geo: &Arc<Mutex<HomeGeoConfig>>,
    wigle_home: &Arc<Mutex<Option<[u8; 6]>>>,
    op: NvsDeferOp,
) -> Result<(), String> {
    match op {
        NvsDeferOp::HomeGeoClear => {
            config::home_geo_clear(nvs).map_err(|e| format!("{e:#}"))?;
            if let Ok(mut g) = home_geo.lock() {
                *g = HomeGeoConfig::default();
            }
            web_snapshots_bump_forms_rev();
            Ok(())
        }
        NvsDeferOp::HomeGeoSave { lat, lon, radius_m } => {
            config::home_geo_save(nvs, lat, lon, radius_m).map_err(|e| format!("{e:#}"))?;
            let loaded = config::home_geo_load(nvs).unwrap_or_default();
            if let Ok(mut g) = home_geo.lock() {
                *g = loaded;
            }
            web_snapshots_bump_forms_rev();
            Ok(())
        }
        NvsDeferOp::FleetRosterWriteBlob(blob) => {
            fleet_roster::write_roster_blob(nvs, &blob).map_err(|e| format!("{e:#}"))
        }
        NvsDeferOp::SaveStaSetup {
            sta_ssid,
            sta_pass,
            ap_ssid,
            ap_pass,
            scan_ble,
            upload_on_boot,
        } => {
            let prev = Config::load(nvs).map_err(|e| format!("{e:#}"))?;
            let clear_home_bssid = if prev.provisioning_complete() {
                let old = prev.sta_ssid.as_deref().map(str::trim);
                old != Some(sta_ssid.trim())
            } else {
                false
            };
            let cfg = Config::with_sta_config(
                &prev,
                sta_ssid,
                sta_pass,
                scan_ble,
                upload_on_boot,
                ap_ssid,
                ap_pass,
            )
            .map_err(|e| e.to_string())?;
            if clear_home_bssid {
                config::wigle_home_bssid_clear(nvs).map_err(|e| format!("{e:#}"))?;
                if let Ok(mut g) = wigle_home.lock() {
                    *g = None;
                }
            }
            cfg.save(nvs).map_err(|e| format!("{e:#}"))?;
            stats::set_management_ap_ssid(cfg.resolved_ap_ssid());
            stats::set_nvs_config_flags(
                cfg.wigle_configured(),
                cfg.wdgwars_configured(),
                cfg.provisioning_complete(),
            );
            web_snapshots_bump_forms_rev();
            request_pending_provisioning_reboot();
            Ok(())
        }
        NvsDeferOp::SaveUploadCredentials {
            wg_name,
            wg_token,
            wd_key,
            wd_clear,
        } => {
            let prev = Config::load(nvs).map_err(|e| format!("{e:#}"))?;
            let cfg =
                Config::with_uploads_credentials(&prev, &wg_name, &wg_token, &wd_key, wd_clear);
            cfg.save(nvs).map_err(|e| format!("{e:#}"))?;
            stats::set_nvs_config_flags(
                cfg.wigle_configured(),
                cfg.wdgwars_configured(),
                cfg.provisioning_complete(),
            );
            web_snapshots_set_wigle_api_name(cfg.wigle_api_name.as_deref().unwrap_or(""));
            Ok(())
        }
        NvsDeferOp::SaveAlertSound {
            mask,
            volume_percent,
        } => {
            config::alert_sound_mask_save(nvs, mask).map_err(|e| format!("{e:#}"))?;
            config::alert_buzzer_volume_save(nvs, volume_percent).map_err(|e| format!("{e:#}"))?;
            stats::set_alert_sound_mask(mask);
            stats::set_alert_buzzer_volume_percent(volume_percent);
            Ok(())
        }
    }
}
