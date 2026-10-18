//! Persistent configuration held in NVS partition namespace `wardriver`.
//!
//! Provisioning writes to this store. Boots read it to decide whether first-boot
//! provisioning is done, to connect to the home network, and optionally upload
//! pending WiGLE CSVs when API credentials are set. Secrets live only here
//! and never in the image.
//!
//! WiGLE privacy: optional NVS blobs store the learned home STA BSSID and a
//! manual list of BSSIDs to exclude from CSV / sniffer reports.
//!
//! GPS privacy zone: optional NVS keys store a lat/lon POI and radius; sniffers enter
//! scan-only mode (detection RF only, no wardrive capture) while inside that circle.

use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::{info, warn};

const NS: &str = "wardriver";

const K_STA_SSID: &str = "sta_ssid";
const K_STA_PASS: &str = "sta_pass";
const K_AP_SSID: &str = "ap_ssid";
const K_AP_PASS: &str = "ap_pass";
const K_WIGLE_NAME: &str = "wg_name";
const K_WIGLE_TOKEN: &str = "wg_token";
const K_WDGWARS_KEY: &str = "wd_key";
const K_SCAN_BLE: &str = "scan_ble";
const K_UPLOAD_ON_BOOT: &str = "upload_boot";
/// Per-event buzzer enable bitmask ([`crate::alert_feedback::AlertKind`] bit index).
const K_ALERT_SND_MASK: &str = "alert_snd";
/// Buzzer LEDC duty 0–100 % (shield passive buzzer on GPIO7).
const K_ALERT_BUZZER_VOL: &str = "alert_vol";

/// Default buzzer volume when NVS key is missing.
pub const DEFAULT_ALERT_BUZZER_VOLUME_PERCENT: u8 = 85;

/// Learned home AP BSSID (6 raw bytes) when WiGLE upload is configured.
pub const K_WIGLE_HOME_BSSID: &str = "wg_home_bssid";
/// Manual exclusion list: first byte = count (0–32), then `count` × 6 bytes.
pub const K_WIGLE_EXCLUDE_MACS: &str = "wg_exclude_macs";

/// GPS privacy zone: 16 bytes = `lat` (f64 LE) + `lon` (f64 LE), WGS84 degrees.
pub const K_HOME_GEO_POI: &str = "home_geo_poi";
/// Optional radius in meters (`u32` NVS type). Absent → [`DEFAULT_HOME_GEO_RADIUS_M`].
pub const K_HOME_GEO_RADIUS_M: &str = "home_geo_rad";

/// Default ~0.5 statute mile (1609.344 / 2), rounded up.
pub const DEFAULT_HOME_GEO_RADIUS_M: u32 = 805;

/// Max radius to accept from UI (sanity cap).
pub const MAX_HOME_GEO_RADIUS_M: u32 = 200_000;

/// Min radius from UI.
pub const MIN_HOME_GEO_RADIUS_M: u32 = 10;

/// Max MAC addresses in the merged denylist (ESP-NOW single-frame budget).
pub const MAX_WIGLE_DENYLIST_MACS: usize = 32;

/// Max manual SSIDs in the WiGLE SSID denylist (must also fit one ESP-NOW frame).
pub const MAX_WIGLE_DENYLIST_SSIDS: usize = 32;

/// Max UTF-8 bytes per excluded SSID (802.11 SSID element limit).
pub const MAX_WIGLE_DENYLIST_SSID_BYTES: usize = 32;

const MAX_SSID_LEN: usize = 33;
const MAX_PASSWORD_LEN: usize = 65;
const MAX_API_LEN: usize = 96;

/// Compile-time fleet management SoftAP WPA2 passphrase.
pub use wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_PASS as DEFAULT_CONTROL_AP_PASS;
/// Compile-time fleet management SoftAP SSID (`.fleet/mgmt_ap.toml` / build.rs).
pub use wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_SSID as DEFAULT_CONTROL_AP_SSID;

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub sta_ssid: Option<String>,
    pub sta_password: Option<String>,
    /// Dashboard / provisioning SoftAP SSID (distinct from home STA). Empty/missing NVS → compile-time [`DEFAULT_CONTROL_AP_SSID`] at runtime.
    pub ap_ssid: Option<String>,
    /// WPA2-PSK for management SoftAP; empty/missing NVS → compile-time [`DEFAULT_CONTROL_AP_PASS`]. Must be 8–63 chars when stored; open AP is not supported.
    pub ap_password: Option<String>,
    pub wigle_api_name: Option<String>,
    pub wigle_api_token: Option<String>,
    /// API key for [wdgwars.pl](https://wdgwars.pl) uploads (`X-API-Key`).
    pub wdgwars_api_key: Option<String>,
    pub scan_ble: bool,
    pub upload_on_boot: bool,
}

impl Config {
    pub fn load(partition: &EspNvsPartition<NvsDefault>) -> Result<Self> {
        let nvs =
            EspNvs::new(partition.clone(), NS, true).context("open NVS namespace for read")?;

        let sta_ssid = read_str(&nvs, K_STA_SSID, MAX_SSID_LEN);
        let sta_password = read_str(&nvs, K_STA_PASS, MAX_PASSWORD_LEN);
        let ap_ssid = read_str(&nvs, K_AP_SSID, MAX_SSID_LEN);
        let ap_password = read_str(&nvs, K_AP_PASS, MAX_PASSWORD_LEN);
        let wigle_api_name = read_str(&nvs, K_WIGLE_NAME, MAX_API_LEN);
        let wigle_api_token = read_str(&nvs, K_WIGLE_TOKEN, MAX_API_LEN);
        let wdgwars_api_key = read_str(&nvs, K_WDGWARS_KEY, MAX_API_LEN);
        let scan_ble = nvs.get_u8(K_SCAN_BLE).unwrap_or(None).unwrap_or(0) != 0;
        let upload_on_boot = nvs.get_u8(K_UPLOAD_ON_BOOT).unwrap_or(None).unwrap_or(1) != 0;

        let cfg = Self {
            sta_ssid,
            sta_password,
            ap_ssid,
            ap_password,
            wigle_api_name,
            wigle_api_token,
            wdgwars_api_key,
            scan_ble,
            upload_on_boot,
        };

        info!(
            "config: provisioning_complete={} wigle_configured={} wdgwars_configured={} ssid={:?} scan_ble={} upload_on_boot={}",
            cfg.provisioning_complete(),
            cfg.wigle_configured(),
            cfg.wdgwars_configured(),
            cfg.sta_ssid.as_deref(),
            cfg.scan_ble,
            cfg.upload_on_boot,
        );

        Ok(cfg)
    }

    pub fn save(&self, partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
        let mut nvs =
            EspNvs::new(partition.clone(), NS, true).context("open NVS namespace for write")?;

        write_str(&mut nvs, K_STA_SSID, self.sta_ssid.as_deref())?;
        write_str(&mut nvs, K_STA_PASS, self.sta_password.as_deref())?;
        write_str(&mut nvs, K_AP_SSID, self.ap_ssid.as_deref())?;
        write_str(&mut nvs, K_AP_PASS, self.ap_password.as_deref())?;
        write_str(&mut nvs, K_WIGLE_NAME, self.wigle_api_name.as_deref())?;
        write_str(&mut nvs, K_WIGLE_TOKEN, self.wigle_api_token.as_deref())?;
        write_str(&mut nvs, K_WDGWARS_KEY, self.wdgwars_api_key.as_deref())?;
        nvs.set_u8(K_SCAN_BLE, if self.scan_ble { 1 } else { 0 })?;
        nvs.set_u8(K_UPLOAD_ON_BOOT, if self.upload_on_boot { 1 } else { 0 })?;
        info!("config: saved to NVS");
        Ok(())
    }

    /// True when home WiFi SSID is set so the device can leave first-boot provisioning.
    pub fn provisioning_complete(&self) -> bool {
        is_nonempty(&self.sta_ssid)
    }

    /// True when both WiGLE API name and token are set (uploads may run).
    pub fn wigle_configured(&self) -> bool {
        is_nonempty(&self.wigle_api_name) && is_nonempty(&self.wigle_api_token)
    }

    /// True when a Watch Dogs Go Wars API key is set.
    pub fn wdgwars_configured(&self) -> bool {
        is_nonempty(&self.wdgwars_api_key)
    }

    /// Resolved management SoftAP SSID for WiFi config and UI (never empty).
    pub fn resolved_ap_ssid(&self) -> &str {
        self.ap_ssid
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_CONTROL_AP_SSID)
    }

    /// Resolved WPA2 passphrase for management SoftAP (8–63 chars; never open).
    pub fn resolved_ap_password(&self) -> &str {
        self.ap_password
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(DEFAULT_CONTROL_AP_PASS)
    }

    /// `POST /setup` — home WiFi + scan options + management SoftAP. Preserves WiGLE and WDGoWars credentials from `prev`.
    ///
    /// Management AP: SSID required (non-empty). Password: 8–63 chars for WPA2, or blank to keep
    /// [`Config::ap_password`] from `prev` (first save: blank → default passphrase at runtime, NVS unset).
    pub fn with_sta_config(
        prev: &Config,
        sta_ssid: String,
        sta_pass: String,
        scan_ble: bool,
        upload_on_boot: bool,
        ap_ssid: String,
        ap_pass: String,
    ) -> anyhow::Result<Self> {
        let sta_ssid = sta_ssid.trim().to_owned();
        if sta_ssid.is_empty() {
            anyhow::bail!("missing sta_ssid");
        }
        let ap_ssid_t = ap_ssid.trim().to_owned();
        if ap_ssid_t.is_empty() {
            anyhow::bail!("management SoftAP SSID is required");
        }
        if ap_ssid_t.len() > 32 {
            anyhow::bail!("management SoftAP SSID too long");
        }
        let ap_pass_t = ap_pass.trim();
        let ap_password = if ap_pass_t.is_empty() {
            prev.ap_password.clone()
        } else {
            let len = ap_pass_t.len();
            if len < 8 || len > 63 {
                anyhow::bail!(
                    "management SoftAP password must be 8–63 characters (WPA2), or leave blank to keep the current password"
                );
            }
            Some(ap_pass_t.to_owned())
        };
        Ok(Self {
            sta_ssid: Some(sta_ssid),
            sta_password: Some(sta_pass),
            ap_ssid: Some(ap_ssid_t),
            ap_password,
            wigle_api_name: prev.wigle_api_name.clone(),
            wigle_api_token: prev.wigle_api_token.clone(),
            wdgwars_api_key: prev.wdgwars_api_key.clone(),
            scan_ble,
            upload_on_boot,
        })
    }

    /// `POST /uploads/save` — merge new upload credentials.
    ///
    /// - Empty or whitespace `wg_name` → clear both WiGLE `wigle_api_*` fields.
    /// - When `wg_name` is set, an empty `wg_token` string keeps the previous API token in NVS.
    /// - `wd_clear` → remove WDGoWars key.
    /// - When not clearing, a non-empty `wd_key` replaces; an empty/whitespace `wd_key` leaves the
    ///   previous WDGoWars key unchanged.
    pub fn with_uploads_credentials(
        prev: &Config,
        wg_name: &str,
        wg_token: &str,
        wd_key: &str,
        wd_clear: bool,
    ) -> Self {
        let (wigle_api_name, wigle_api_token) = if wg_name.trim().is_empty() {
            (None, None)
        } else {
            let n = Some(wg_name.trim().to_owned());
            let t = if wg_token.trim().is_empty() {
                prev.wigle_api_token.clone()
            } else {
                Some(wg_token.trim().to_owned())
            };
            (n, t)
        };
        let wdgwars_api_key = if wd_clear {
            None
        } else if wd_key.trim().is_empty() {
            prev.wdgwars_api_key.clone()
        } else {
            Some(wd_key.trim().to_owned())
        };
        Self {
            sta_ssid: prev.sta_ssid.clone(),
            sta_password: prev.sta_password.clone(),
            ap_ssid: prev.ap_ssid.clone(),
            ap_password: prev.ap_password.clone(),
            wigle_api_name,
            wigle_api_token,
            wdgwars_api_key,
            scan_ble: prev.scan_ble,
            upload_on_boot: prev.upload_on_boot,
        }
    }
}

/// Load learned home AP BSSID (6 bytes), if present and valid.
/// Load alert sound bitmask from NVS (defaults to all enabled).
pub fn alert_sound_mask_load(partition: &EspNvsPartition<NvsDefault>) -> u8 {
    let Ok(nvs) = EspNvs::new(partition.clone(), NS, true) else {
        return crate::alert_feedback::ALERT_SOUND_MASK_ALL;
    };
    let raw = nvs.get_u8(K_ALERT_SND_MASK).ok().flatten().unwrap_or(0xFF);
    if raw == 0xFF {
        crate::alert_feedback::ALERT_SOUND_MASK_ALL
    } else {
        raw & crate::alert_feedback::ALERT_SOUND_MASK_ALL
    }
}

/// Persist alert sound bitmask (only bits 0..5 are used).
pub fn alert_sound_mask_save(partition: &EspNvsPartition<NvsDefault>, mask: u8) -> Result<()> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for alert_snd write")?;
    let m = mask & crate::alert_feedback::ALERT_SOUND_MASK_ALL;
    nvs.set_u8(K_ALERT_SND_MASK, m)
        .map_err(|e| anyhow::anyhow!("nvs set_u8 alert_snd: {e:?}"))?;
    info!("config: saved alert_snd_mask=0x{m:02x}");
    Ok(())
}

/// Load buzzer volume percent from NVS (0–100).
pub fn alert_buzzer_volume_load(partition: &EspNvsPartition<NvsDefault>) -> u8 {
    let Ok(nvs) = EspNvs::new(partition.clone(), NS, true) else {
        return DEFAULT_ALERT_BUZZER_VOLUME_PERCENT;
    };
    match nvs.get_u8(K_ALERT_BUZZER_VOL).ok().flatten() {
        None | Some(0xFF) => DEFAULT_ALERT_BUZZER_VOLUME_PERCENT,
        Some(v) => v.min(100),
    }
}

pub fn alert_buzzer_volume_save(
    partition: &EspNvsPartition<NvsDefault>,
    percent: u8,
) -> Result<()> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for alert_vol write")?;
    let p = percent.min(100);
    nvs.set_u8(K_ALERT_BUZZER_VOL, p)
        .map_err(|e| anyhow::anyhow!("nvs set_u8 alert_vol: {e:?}"))?;
    info!("config: saved alert_buzzer_volume={p}%");
    Ok(())
}

pub fn wigle_home_bssid_load(partition: &EspNvsPartition<NvsDefault>) -> Result<Option<[u8; 6]>> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for wg_home_bssid")?;
    let mut buf = [0u8; 6];
    match nvs.get_blob(K_WIGLE_HOME_BSSID, &mut buf) {
        Ok(Some(slice)) if slice.len() == 6 => {
            let mut m = [0u8; 6];
            m.copy_from_slice(slice);
            Ok(Some(m))
        }
        Ok(Some(_)) => {
            warn!("wg_home_bssid: wrong length; ignoring");
            Ok(None)
        }
        Ok(None) => Ok(None),
        Err(e) => {
            warn!("wg_home_bssid read: {e:?}");
            Ok(None)
        }
    }
}

pub fn wigle_home_bssid_save(
    partition: &EspNvsPartition<NvsDefault>,
    bssid: &[u8; 6],
) -> Result<()> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for wg_home_bssid")?;
    nvs.set_blob(K_WIGLE_HOME_BSSID, bssid.as_slice())
        .map_err(|e| anyhow::anyhow!("nvs set_blob wg_home_bssid: {e:?}"))?;
    info!(
        "config: saved wg_home_bssid {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bssid[0], bssid[1], bssid[2], bssid[3], bssid[4], bssid[5]
    );
    Ok(())
}

pub fn wigle_home_bssid_clear(partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for wg_home_bssid")?;
    match nvs.remove(K_WIGLE_HOME_BSSID) {
        Ok(true) => info!("config: cleared wg_home_bssid"),
        Ok(false) => {}
        Err(e) => warn!("wg_home_bssid remove: {e:?}"),
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeGeoConfig {
    pub poi: Option<(i64, i64)>,
    pub radius_m: u32,
}

impl Default for HomeGeoConfig {
    fn default() -> Self {
        Self {
            poi: None,
            radius_m: DEFAULT_HOME_GEO_RADIUS_M,
        }
    }
}

fn home_geo_coords_valid(lat: f64, lon: f64) -> bool {
    lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0
}

/// Micro-degrees fixed point so `HomeGeoConfig` stays `Eq` without float quirks in tests.
fn deg_to_microdeg(d: f64) -> i64 {
    (d * 1_000_000.0).round() as i64
}

fn microdeg_to_deg(m: i64) -> f64 {
    m as f64 / 1_000_000.0
}

/// Load GPS privacy zone (optional POI + radius). Invalid blobs are ignored.
pub fn home_geo_load(partition: &EspNvsPartition<NvsDefault>) -> Result<HomeGeoConfig> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for home_geo")?;
    let mut buf = [0u8; 16];
    let poi = match nvs.get_blob(K_HOME_GEO_POI, &mut buf) {
        Ok(Some(s)) if s.len() == 16 => {
            let lat = f64::from_le_bytes(s[0..8].try_into().unwrap());
            let lon = f64::from_le_bytes(s[8..16].try_into().unwrap());
            if home_geo_coords_valid(lat, lon) {
                Some((deg_to_microdeg(lat), deg_to_microdeg(lon)))
            } else {
                warn!("home_geo_poi: invalid lat/lon; ignoring");
                None
            }
        }
        Ok(Some(_)) => {
            warn!("home_geo_poi: wrong blob length; ignoring");
            None
        }
        Ok(None) => None,
        Err(e) => {
            warn!("home_geo_poi read: {e:?}");
            None
        }
    };

    let radius_m = match nvs.get_u32(K_HOME_GEO_RADIUS_M) {
        Ok(Some(r)) if (MIN_HOME_GEO_RADIUS_M..=MAX_HOME_GEO_RADIUS_M).contains(&r) => r,
        Ok(Some(r)) => {
            warn!("home_geo_rad: value {r} out of range; using default");
            DEFAULT_HOME_GEO_RADIUS_M
        }
        Ok(None) => DEFAULT_HOME_GEO_RADIUS_M,
        Err(e) => {
            warn!("home_geo_rad read: {e:?}");
            DEFAULT_HOME_GEO_RADIUS_M
        }
    };

    Ok(HomeGeoConfig { poi, radius_m })
}

pub fn home_geo_save(
    partition: &EspNvsPartition<NvsDefault>,
    lat: f64,
    lon: f64,
    radius_m: u32,
) -> Result<()> {
    if !home_geo_coords_valid(lat, lon) {
        bail!("invalid latitude or longitude");
    }
    if !(MIN_HOME_GEO_RADIUS_M..=MAX_HOME_GEO_RADIUS_M).contains(&radius_m) {
        bail!("radius must be between {MIN_HOME_GEO_RADIUS_M} and {MAX_HOME_GEO_RADIUS_M} meters");
    }
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for home_geo write")?;
    let mut blob = [0u8; 16];
    blob[0..8].copy_from_slice(&lat.to_le_bytes());
    blob[8..16].copy_from_slice(&lon.to_le_bytes());
    nvs.set_blob(K_HOME_GEO_POI, &blob)
        .map_err(|e| anyhow::anyhow!("nvs set_blob home_geo_poi: {e:?}"))?;
    nvs.set_u32(K_HOME_GEO_RADIUS_M, radius_m)
        .map_err(|e| anyhow::anyhow!("nvs set_u32 home_geo_rad: {e:?}"))?;
    info!("config: saved home_geo_poi lat={lat:.7} lon={lon:.7} radius_m={radius_m}");
    Ok(())
}

pub fn home_geo_clear(partition: &EspNvsPartition<NvsDefault>) -> Result<()> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for home_geo clear")?;
    for k in [K_HOME_GEO_POI, K_HOME_GEO_RADIUS_M] {
        match nvs.remove(k) {
            Ok(true) => {}
            Ok(false) => {} // key absent — idempotent clear
            Err(e) => return Err(anyhow::anyhow!("nvs remove {k}: {e:?}")),
        }
    }
    info!("config: cleared home zone");
    Ok(())
}

/// Parse decimal degree strings; empty `radius_s` → [`DEFAULT_HOME_GEO_RADIUS_M`].
pub fn parse_home_geo_form(
    lat_s: &str,
    lon_s: &str,
    radius_s: &str,
) -> Result<(f64, f64, u32), String> {
    let lat = lat_s
        .trim()
        .parse::<f64>()
        .map_err(|_| "invalid latitude".to_string())?;
    let lon = lon_s
        .trim()
        .parse::<f64>()
        .map_err(|_| "invalid longitude".to_string())?;
    let radius_m = if radius_s.trim().is_empty() {
        DEFAULT_HOME_GEO_RADIUS_M
    } else {
        radius_s
            .trim()
            .parse::<u32>()
            .map_err(|_| "invalid radius (meters)".to_string())?
    };
    if !home_geo_coords_valid(lat, lon) {
        return Err("latitude must be [-90,90] and longitude [-180,180]".to_string());
    }
    if !(MIN_HOME_GEO_RADIUS_M..=MAX_HOME_GEO_RADIUS_M).contains(&radius_m) {
        return Err(format!(
            "radius must be {MIN_HOME_GEO_RADIUS_M}..={MAX_HOME_GEO_RADIUS_M} meters"
        ));
    }
    Ok((lat, lon, radius_m))
}

/// Same GPS quality + HDOP rules as the wardriving scan gate (fleet sniff + WiGLE rows).
pub fn home_geo_fix_meets_scan_gate(fix: &crate::gps::Fix) -> bool {
    crate::wifi_scan::is_fix_usable_for_location(fix)
}

pub fn home_geo_poi_degrees(cfg: &HomeGeoConfig) -> Option<(f64, f64)> {
    cfg.poi
        .map(|(la, lo)| (microdeg_to_deg(la), microdeg_to_deg(lo)))
}

/// Load manual exclusion MACs from NVS (up to [`MAX_WIGLE_DENYLIST_MACS`]).
pub fn wigle_exclude_macs_load(
    partition: &EspNvsPartition<NvsDefault>,
) -> Result<HashSet<[u8; 6]>> {
    let nvs = EspNvs::new(partition.clone(), NS, true).context("open NVS for wg_exclude_macs")?;
    let mut raw = [0u8; 1 + 6 * MAX_WIGLE_DENYLIST_MACS];
    let slice = match nvs.get_blob(K_WIGLE_EXCLUDE_MACS, &mut raw) {
        Ok(Some(s)) => s,
        Ok(None) => return Ok(HashSet::new()),
        Err(e) => {
            warn!("wg_exclude_macs read: {e:?}");
            return Ok(HashSet::new());
        }
    };
    if slice.is_empty() {
        return Ok(HashSet::new());
    }
    let n = slice[0] as usize;
    if n > MAX_WIGLE_DENYLIST_MACS {
        warn!("wg_exclude_macs: count {n} too high; ignoring");
        return Ok(HashSet::new());
    }
    if slice.len() != 1 + 6 * n {
        warn!("wg_exclude_macs: blob length mismatch; ignoring");
        return Ok(HashSet::new());
    }
    let mut out = HashSet::with_capacity(n);
    for i in 0..n {
        let o = 1 + i * 6;
        let mut m = [0u8; 6];
        m.copy_from_slice(&slice[o..o + 6]);
        out.insert(m);
    }
    Ok(out)
}

/// Parse one MAC: `aa:bb:cc:dd:ee:ff`, dashes, or 12 hex digits (case-insensitive).
pub fn parse_mac_bytes(s: &str) -> Option<[u8; 6]> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let hex_only: String = t
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if hex_only.len() != 12 {
        return None;
    }
    let mut out = [0u8; 6];
    for i in 0..6 {
        let byte = u8::from_str_radix(&hex_only[i * 2..i * 2 + 2], 16).ok()?;
        out[i] = byte;
    }
    Some(out)
}

/// Parse textarea: one MAC per line. Dedupes, caps at [`MAX_WIGLE_DENYLIST_MACS`].
/// Returns error if any non-empty line is not a valid MAC.
pub fn parse_exclude_mac_lines(text: &str) -> Result<HashSet<[u8; 6]>, String> {
    let mut out = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let m = parse_mac_bytes(line).ok_or_else(|| format!("invalid MAC: {line}"))?;
        out.insert(m);
        if out.len() > MAX_WIGLE_DENYLIST_MACS {
            return Err(format!(
                "more than {} unique MAC addresses",
                MAX_WIGLE_DENYLIST_MACS
            ));
        }
    }
    Ok(out)
}

/// Merge SoftAP, optional home, and manual exclusions; cap at [`MAX_WIGLE_DENYLIST_MACS`].
/// Order: SoftAP, home, then manual (sorted) for stable truncation.
pub fn merged_wigle_denylist_mac_vec(
    softap: Option<[u8; 6]>,
    home: Option<[u8; 6]>,
    manual: &HashSet<[u8; 6]>,
) -> Vec<[u8; 6]> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for m in softap.into_iter().chain(home) {
        if seen.insert(m) {
            out.push(m);
            if out.len() >= MAX_WIGLE_DENYLIST_MACS {
                return out;
            }
        }
    }
    let mut keys: Vec<[u8; 6]> = manual.iter().copied().collect();
    keys.sort();
    for m in keys {
        if seen.insert(m) {
            out.push(m);
            if out.len() >= MAX_WIGLE_DENYLIST_MACS {
                break;
            }
        }
    }
    out
}

pub fn bssid_in_denylist(
    bssid: &[u8; 6],
    softap: &Option<[u8; 6]>,
    home: &Option<[u8; 6]>,
    manual: &HashSet<[u8; 6]>,
) -> bool {
    softap.as_ref() == Some(bssid) || home.as_ref() == Some(bssid) || manual.contains(bssid)
}

/// Parse textarea: one SSID per line. Dedupes, caps at [`MAX_WIGLE_DENYLIST_SSIDS`].
pub fn parse_exclude_ssid_lines(text: &str) -> Result<HashSet<String>, String> {
    let mut out = HashSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.as_bytes().len() > MAX_WIGLE_DENYLIST_SSID_BYTES {
            return Err(format!(
                "SSID too long (max {} bytes): {line}",
                MAX_WIGLE_DENYLIST_SSID_BYTES
            ));
        }
        out.insert(line.to_string());
        if out.len() > MAX_WIGLE_DENYLIST_SSIDS {
            return Err(format!(
                "more than {} unique SSIDs",
                MAX_WIGLE_DENYLIST_SSIDS
            ));
        }
    }
    Ok(out)
}

pub fn ssid_in_denylist(ssid: &str, manual: &HashSet<String>) -> bool {
    manual.contains(ssid)
}

fn is_nonempty(s: &Option<String>) -> bool {
    s.as_deref().map(|v| !v.is_empty()).unwrap_or(false)
}

fn read_str(nvs: &EspNvs<NvsDefault>, key: &str, cap: usize) -> Option<String> {
    let mut buf = vec![0u8; cap];
    match nvs.get_str(key, &mut buf) {
        Ok(Some(s)) => Some(s.to_owned()),
        Ok(None) => None,
        Err(e) => {
            warn!("read {key}: {e}");
            None
        }
    }
}

fn write_str(nvs: &mut EspNvs<NvsDefault>, key: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(v) => nvs
            .set_str(key, v)
            .with_context(|| format!("nvs set {key}")),
        None => {
            let _ = nvs.remove(key);
            Ok(())
        }
    }
}
