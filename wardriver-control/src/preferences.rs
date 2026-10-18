//! Control-node preferences on SD (`wolfPACK/config/preferences.json`).
//!
//! **In-memory cache:** After [`load_or_default`] runs (boot, SD mounted), the full JSON document is
//! kept in [`PREF_JSON_CACHE`]. [`persist_from_stats`] merges into that cache and writes SD without
//! re-reading the file each time (unknown keys are preserved). External edits on the card while
//! running are ignored until reboot.

use std::sync::Mutex;

use anyhow::{Context, Result};
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::storage::{self, SdIoHandle};
use wardriver_core::detection::{
    detection_ble_disable_bit, DETECTION_BLE_DISABLE_MASK_DEFAULT,
    DETECTION_WIFI_DISABLE_MASK_DEFAULT,
};
use wardriver_stats as stats;

static PREF_JSON_CACHE: Mutex<Option<Value>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPreferences {
    #[serde(default = "default_prefs_v")]
    pub v: u32,
    #[serde(
        default = "default_detection_disable_wifi_mask",
        alias = "flock_disable_wifi_mask"
    )]
    pub detection_disable_wifi_mask: u32,
    #[serde(
        default = "default_detection_disable_ble_mask",
        alias = "flock_disable_ble_mask"
    )]
    pub detection_disable_ble_mask: u32,
    #[serde(default = "default_remote_id_disable_mask")]
    pub remote_id_disable_mask: u32,
    /// Legacy wolfPACK field: bit 0 disabled Axon when method IDs used the `96+` range. Merged into
    /// [`detection_disable_ble_mask`] bit for method **35** on load.
    #[serde(default, alias = "surveillance_disable_ble_mask")]
    pub legacy_surveillance_disable_ble_mask: u32,
    /// BLE-role sniffers: active LE scan (SCAN_REQ) when true; passive when false.
    #[serde(default)]
    pub ble_active_scan: bool,
    #[serde(default = "default_diag_log_enabled")]
    pub diag_log_enabled: bool,
}

fn default_prefs_v() -> u32 {
    1
}

fn default_diag_log_enabled() -> bool {
    true
}

fn default_detection_disable_wifi_mask() -> u32 {
    DETECTION_WIFI_DISABLE_MASK_DEFAULT
}

fn default_detection_disable_ble_mask() -> u32 {
    DETECTION_BLE_DISABLE_MASK_DEFAULT
}

fn default_remote_id_disable_mask() -> u32 {
    0
}

impl Default for ControlPreferences {
    fn default() -> Self {
        Self {
            v: 1,
            detection_disable_wifi_mask: default_detection_disable_wifi_mask(),
            detection_disable_ble_mask: default_detection_disable_ble_mask(),
            remote_id_disable_mask: default_remote_id_disable_mask(),
            legacy_surveillance_disable_ble_mask: 0,
            ble_active_scan: false,
            diag_log_enabled: true,
        }
    }
}

/// PACK: Axon (method 35) uses `detection_disable_ble_mask`; merge pre-refactor `surveillance_disable_ble_mask`.
fn lpr_camera_ble_mask_with_legacy_merge(p: &ControlPreferences) -> u32 {
    let mut ble = p.detection_disable_ble_mask;
    if p.legacy_surveillance_disable_ble_mask & 1 != 0 {
        ble |= detection_ble_disable_bit(
            wardriver_core::detection::MOBILE_SURVEILLANCE_AXON_METHOD_ID,
        );
    }
    ble
}

/// Apply preferences from a worker-collected boot snapshot (no SD VFS on main).
pub fn apply_from_boot_snapshot(text: Option<&str>) -> ControlPreferences {
    let path = storage::control_preferences_path();
    let val: Value = match text {
        Some(t) => match serde_json::from_str::<Value>(t) {
            Ok(v) => v,
            Err(e) => {
                warn!("preferences: parse JSON {}: {e}", path.display());
                Value::Object(Default::default())
            }
        },
        None => Value::Object(Default::default()),
    };

    if let Ok(mut g) = PREF_JSON_CACHE.lock() {
        *g = Some(val.clone());
    }

    match serde_json::from_value::<ControlPreferences>(val) {
        Ok(p) => p,
        Err(e) => {
            warn!("preferences: deserialize: {e}");
            ControlPreferences::default()
        }
    }
}

/// Load preferences from SD via the sd-io worker (runtime callers).
pub fn load_or_default(sd_io: &SdIoHandle) -> ControlPreferences {
    let val: Value = match storage::read_preferences_text(sd_io) {
        Ok(Some(text)) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    "preferences: parse JSON {}: {e}",
                    storage::control_preferences_path().display()
                );
                Value::Object(Default::default())
            }
        },
        Ok(None) => Value::Object(Default::default()),
        Err(e) => {
            warn!("preferences: SD read failed: {e}");
            Value::Object(Default::default())
        }
    };

    if let Ok(mut g) = PREF_JSON_CACHE.lock() {
        *g = Some(val.clone());
    }

    match serde_json::from_value::<ControlPreferences>(val) {
        Ok(p) => p,
        Err(e) => {
            warn!("preferences: deserialize ControlPreferences: {e}");
            ControlPreferences::default()
        }
    }
}

pub fn apply_to_stats(p: &ControlPreferences) {
    stats::set_detection_disable_masks(
        p.detection_disable_wifi_mask,
        lpr_camera_ble_mask_with_legacy_merge(p),
    );
    stats::set_remote_id_disable_mask(p.remote_id_disable_mask);
    stats::set_ble_active_scan(p.ble_active_scan);
    stats::set_diag_log_enabled(p.diag_log_enabled);
}

/// Merge runtime stats into the cached JSON document and atomically write via sd-io.
pub fn persist_from_stats(sd_io: &SdIoHandle) -> Result<()> {
    if !stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
        anyhow::bail!("SD not mounted");
    }

    let body = {
        let mut guard = PREF_JSON_CACHE.lock().unwrap();
        if guard.is_none() {
            match storage::read_preferences_text(sd_io) {
                Ok(Some(text)) => {
                    let v = serde_json::from_str(&text)
                        .unwrap_or_else(|_| Value::Object(Default::default()));
                    *guard = Some(v);
                }
                Ok(None) => {
                    *guard = Some(Value::Object(Default::default()));
                }
                Err(e) => {
                    warn!("preferences: cold-cache read failed, empty merge: {e:#}");
                    *guard = Some(Value::Object(Default::default()));
                }
            }
        }

        let val = guard.as_mut().expect("preferences cache set above");

        let wifi_mask = u64::from(stats::detection_disable_wifi_mask());
        let ble_mask = u64::from(stats::detection_disable_ble_mask());
        let remote_id_mask = u64::from(stats::remote_id_disable_mask());

        if let Some(obj) = val.as_object_mut() {
            obj.insert("v".to_string(), Value::Number(1.into()));
            obj.insert(
                "detection_disable_wifi_mask".to_string(),
                Value::Number(wifi_mask.into()),
            );
            obj.remove("flock_disable_wifi_mask");
            obj.insert(
                "detection_disable_ble_mask".to_string(),
                Value::Number(ble_mask.into()),
            );
            obj.remove("flock_disable_ble_mask");
            obj.insert(
                "remote_id_disable_mask".to_string(),
                Value::Number(remote_id_mask.into()),
            );
            obj.remove("surveillance_disable_ble_mask");
            obj.insert(
                "ble_active_scan".to_string(),
                Value::Bool(stats::ble_active_scan()),
            );
            obj.insert(
                "diag_log_enabled".to_string(),
                Value::Bool(stats::diag_log_enabled()),
            );
        }

        serde_json::to_string_pretty(val).context("serialize preferences")?
    };

    storage::write_preferences_text(sd_io, &body).context("write preferences to SD")
}
