//! Sniffer-side persistent configuration (NVS).
//!
//! v1 stores only the control node MAC learned via ESP-NOW discovery.

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::{info, warn};
use wardriver_core::detection::{
    DETECTION_BLE_DISABLE_MASK_DEFAULT, DETECTION_WIFI_DISABLE_MASK_DEFAULT,
};

const NS: &str = "sniffer";
const K_CONTROL_MAC: &str = "control_mac";
const K_ROLE: &str = "role";
const K_DETECTION_WIFI_MASK: &str = "detection_wifi_mask";
const K_DETECTION_BLE_MASK: &str = "detection_ble_mask";
/// Legacy NVS keys from the Flock era; migrated into the detection keys on read.
const K_LEGACY_FLOCK_WIFI_MASK: &str = "flock_wifi_mask";
const K_LEGACY_FLOCK_BLE_MASK: &str = "flock_ble_mask";
const K_REMOTE_ID_WIFI_MASK: &str = "rid_wifi_mask";
const K_REMOTE_ID_BLE_MASK: &str = "rid_ble_mask";
const K_BLE_ACTIVE_SCAN: &str = "ble_act_scan";
const K_BOUND_CTRL_MAC: &str = "bound_ctrl_mac";
const K_BOUND_FLEET_SLOT: &str = "bound_fleet_slot";
const K_BOUND_BOOT_ID: &str = "bound_boot_id";

#[derive(Debug, Clone)]
pub struct SnifferConfig {
    pub control_mac: Option<[u8; 6]>,
    /// [`wardriver_proto::SnifferRole`] wire value (default WiFi = 0, Ble = 1).
    pub role: u8,
    /// Per-method WiFi detection disable bitmask from control (`detection_disable_wifi_mask`).
    pub detection_disable_wifi_mask: u32,
    /// Per-method BLE detection disable bitmask from control (`detection_disable_ble_mask`).
    pub detection_disable_ble_mask: u32,
    /// Per-method Remote ID Wi-Fi disable bitmask from control (`remote_id_disable_mask` Wi-Fi bits).
    pub remote_id_disable_wifi_mask: u32,
    /// Per-method Remote ID BLE disable bitmask from control (`remote_id_disable_mask` BLE bits).
    pub remote_id_disable_ble_mask: u32,
    /// Scan-only mode: detection RF only; no wardrive capture or bulk TX.
    pub wardrive_suppressed: bool,
    /// BLE-role: active LE scan (SCAN_REQ) when true; passive when false.
    pub ble_active_scan: bool,
    /// NVS fleet binding: control MAC from adopt welcome (`bound_ctrl_mac`).
    pub bound_control_mac: Option<[u8; 6]>,
    /// Roster slot from last `ControlWelcome` while bound.
    pub bound_fleet_slot: u8,
    /// `control_boot_id` at adopt/bind time.
    pub bound_at_boot_id: u32,
}

impl Default for SnifferConfig {
    fn default() -> Self {
        Self {
            control_mac: None,
            role: 0,
            detection_disable_wifi_mask: DETECTION_WIFI_DISABLE_MASK_DEFAULT,
            detection_disable_ble_mask: DETECTION_BLE_DISABLE_MASK_DEFAULT,
            remote_id_disable_wifi_mask: 0,
            remote_id_disable_ble_mask: 0,
            wardrive_suppressed: false,
            ble_active_scan: false,
            bound_control_mac: None,
            bound_fleet_slot: 0,
            bound_at_boot_id: 0,
        }
    }
}

impl SnifferConfig {
    pub fn load(partition: &EspNvsPartition<NvsDefault>) -> Result<Self> {
        let nvs = EspNvs::new(partition.clone(), NS, true).context("open sniffer NVS")?;
        let control_mac = read_mac(&nvs, K_CONTROL_MAC);
        let role = nvs
            .get_u8(K_ROLE)
            .ok()
            .flatten()
            .filter(|&r| r <= 1)
            .unwrap_or(0);
        let detection_disable_wifi_mask = read_u32_with_legacy_key(
            &nvs,
            K_DETECTION_WIFI_MASK,
            K_LEGACY_FLOCK_WIFI_MASK,
            DETECTION_WIFI_DISABLE_MASK_DEFAULT,
        );
        let detection_disable_ble_mask = read_u32_with_legacy_key(
            &nvs,
            K_DETECTION_BLE_MASK,
            K_LEGACY_FLOCK_BLE_MASK,
            DETECTION_BLE_DISABLE_MASK_DEFAULT,
        );
        let remote_id_disable_wifi_mask = nvs
            .get_u32(K_REMOTE_ID_WIFI_MASK)
            .ok()
            .flatten()
            .unwrap_or(0);
        let remote_id_disable_ble_mask = nvs
            .get_u32(K_REMOTE_ID_BLE_MASK)
            .ok()
            .flatten()
            .unwrap_or(0);
        let ble_active_scan = nvs
            .get_u8(K_BLE_ACTIVE_SCAN)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false);
        let bound_control_mac = read_mac(&nvs, K_BOUND_CTRL_MAC);
        let bound_fleet_slot = nvs.get_u8(K_BOUND_FLEET_SLOT).ok().flatten().unwrap_or(0);
        let bound_at_boot_id = nvs.get_u32(K_BOUND_BOOT_ID).ok().flatten().unwrap_or(0);
        info!(
            "sniffer_config: control_mac={:?} role={} detection_wifi_mask={:#x} detection_ble_mask={:#x} rid_wifi_mask={:#x} rid_ble_mask={:#x} ble_active_scan={}",
            control_mac.map(fmt_mac),
            role,
            detection_disable_wifi_mask,
            detection_disable_ble_mask,
            remote_id_disable_wifi_mask,
            remote_id_disable_ble_mask,
            ble_active_scan as u8
        );
        if let Some(mac) = bound_control_mac {
            info!(
                "sniffer_config: fleet bound mac={} slot={} boot_id={}",
                fmt_mac(mac),
                bound_fleet_slot,
                bound_at_boot_id
            );
        }
        Ok(Self {
            control_mac,
            role,
            detection_disable_wifi_mask,
            detection_disable_ble_mask,
            remote_id_disable_wifi_mask,
            remote_id_disable_ble_mask,
            wardrive_suppressed: false,
            ble_active_scan,
            bound_control_mac,
            bound_fleet_slot,
            bound_at_boot_id,
        })
    }

    pub fn save(&self, partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
        let nvs = EspNvs::new(partition.clone(), NS, true).context("open sniffer NVS write")?;
        if let Some(mac) = self.control_mac {
            nvs.set_blob(K_CONTROL_MAC, &mac)?;
        } else {
            let _ = nvs.remove(K_CONTROL_MAC);
        }
        nvs.set_u8(K_ROLE, self.role.min(1))?;
        nvs.set_u32(K_DETECTION_WIFI_MASK, self.detection_disable_wifi_mask)?;
        nvs.set_u32(K_DETECTION_BLE_MASK, self.detection_disable_ble_mask)?;
        nvs.set_u32(K_REMOTE_ID_WIFI_MASK, self.remote_id_disable_wifi_mask)?;
        nvs.set_u32(K_REMOTE_ID_BLE_MASK, self.remote_id_disable_ble_mask)?;
        nvs.set_u8(K_BLE_ACTIVE_SCAN, u8::from(self.ble_active_scan))?;
        if let Some(mac) = self.bound_control_mac {
            nvs.set_blob(K_BOUND_CTRL_MAC, &mac)?;
            nvs.set_u8(K_BOUND_FLEET_SLOT, self.bound_fleet_slot)?;
            nvs.set_u32(K_BOUND_BOOT_ID, self.bound_at_boot_id)?;
        } else {
            let _ = nvs.remove(K_BOUND_CTRL_MAC);
            let _ = nvs.remove(K_BOUND_FLEET_SLOT);
            let _ = nvs.remove(K_BOUND_BOOT_ID);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_fleet_bound(&self) -> bool {
        self.bound_control_mac.is_some()
    }

    pub fn set_fleet_binding(&mut self, mac: [u8; 6], fleet_slot: u8, control_boot_id: u32) {
        self.bound_control_mac = Some(mac);
        self.bound_fleet_slot = fleet_slot;
        self.bound_at_boot_id = control_boot_id;
        self.control_mac = Some(mac);
    }

    pub fn clear_fleet_binding(&mut self) {
        self.bound_control_mac = None;
        self.bound_fleet_slot = 0;
        self.bound_at_boot_id = 0;
        self.control_mac = None;
    }

    /// True when wardrive capture/buffering is suppressed (detection + alerts only).
    pub fn scan_only(&self) -> bool {
        self.wardrive_suppressed
    }
}

/// Read a u32 NVS value; when the new key is absent, fall back to the legacy key
/// and re-write the value under the new key (erase-optional migration).
fn read_u32_with_legacy_key(
    nvs: &EspNvs<NvsDefault>,
    new_key: &str,
    legacy_key: &str,
    default: u32,
) -> u32 {
    if let Some(v) = nvs.get_u32(new_key).ok().flatten() {
        return v;
    }
    match nvs.get_u32(legacy_key).ok().flatten() {
        Some(v) => {
            if let Err(e) = nvs.set_u32(new_key, v) {
                warn!("sniffer_config: migrate {legacy_key} -> {new_key}: {e}");
            }
            v
        }
        None => default,
    }
}

fn read_mac(nvs: &EspNvs<NvsDefault>, key: &str) -> Option<[u8; 6]> {
    let mut buf = [0u8; 6];
    match nvs.get_blob(key, &mut buf) {
        Ok(Some(_)) => Some(buf),
        Ok(None) => None,
        Err(e) => {
            warn!("sniffer_config: read {key}: {e}");
            None
        }
    }
}

fn fmt_mac(mac: [u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
