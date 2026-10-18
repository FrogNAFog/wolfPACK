//! Passive BLE LPR-camera / mobile-surveillance signals from advertisement heuristics.
//!
//! Uses [`BleEntryV1`] fields from BLE-role sniffers (company ID, truncated name, MAC).
//! Method IDs **32–34** are LPR-camera infrastructure BLE; **35** Axon / **36** i-PRO are mobile
//! surveillance OUIs (also used by Wi-Fi methods **4** / **5**).

use std::collections::{HashMap, HashSet};

use wardriver_proto::BleEntryV1;

use crate::detection::{ble_mask_needs_detection, is_ble_method_disabled, DetectionBleMethod};
use crate::lpr_camera_wifi::LPR_CAMERA_OUIS;

/// Per-source alert cooldown (production: 30 s between alerts for the same advertiser MAC).
pub const PER_SRC_COOLDOWN_MS: u64 = 30_000;

const SRC_IDLE_EVICT_MS: u64 = 60_000;

/// Bluetooth SIG company identifier **0x09C8** (little-endian in adv) — XUNTONG; LPR-camera BLE lineage.
const COMPANY_ID_XUNTONG: u16 = 0x09C8;

/// IEEE OUI **`00:25:DF`** — Axon body cameras (BLE method **35**, Wi-Fi method **4**).
pub const AXON_OUI: [u8; 3] = [0x00, 0x25, 0xDF];

/// IEEE OUI **`D4:2D:C5`** — i-PRO body cameras (BLE method **36**, Wi-Fi method **5**).
pub const IPRO_OUI: [u8; 3] = [0xD4, 0x2D, 0xC5];

/// Check whether `mac` matches the Axon OUI.
///
/// `mac` is a BLE address in **big-endian** byte order (the sniffer reverses
/// NimBLE's LSB-first address in `push_adv_report`), so IEEE OUI bytes are at
/// indices [0..3].
#[inline]
#[must_use]
pub fn is_axon_oui(mac: &[u8; 6]) -> bool {
    mac[0] == AXON_OUI[0] && mac[1] == AXON_OUI[1] && mac[2] == AXON_OUI[2]
}

/// Check whether `mac` matches the i-PRO OUI.
///
/// `mac` is a BLE address in **big-endian** byte order (the sniffer reverses
/// NimBLE's LSB-first address in `push_adv_report`), so IEEE OUI bytes are at
/// indices [0..3].
#[inline]
#[must_use]
pub fn is_ipro_oui(mac: &[u8; 6]) -> bool {
    mac[0] == IPRO_OUI[0] && mac[1] == IPRO_OUI[1] && mac[2] == IPRO_OUI[2]
}

/// True when `mac` (big-endian BLE address) matches a LPR-camera infrastructure OUI.
///
/// The sniffer reverses NimBLE's LSB-first address, so the IEEE OUI bytes are
/// at positions [0..3] — matching the big-endian `LPR_CAMERA_OUIS` table directly.
#[inline]
#[must_use]
fn ble_addr_matches_lpr_camera_oui(mac: &[u8; 6]) -> bool {
    let be_oui: [u8; 3] = [mac[0], mac[1], mac[2]];
    LPR_CAMERA_OUIS.iter().any(|o| *o == be_oui)
}

#[derive(Clone, Copy, Debug, Default)]
struct SrcState {
    last_seen_ms: u64,
    last_alert_ms: u64,
}

pub struct LprCameraBleDetector {
    per_src: HashMap<[u8; 6], SrcState>,
}

impl LprCameraBleDetector {
    pub fn new() -> Self {
        Self {
            per_src: HashMap::new(),
        }
    }

    pub fn prune(&mut self, now_ms: u64) {
        self.per_src
            .retain(|_, s| now_ms.saturating_sub(s.last_seen_ms) <= SRC_IDLE_EVICT_MS);
    }

    pub fn ingest(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        entry: &BleEntryV1,
        ignore: &HashSet<[u8; 6]>,
        disable_ble_mask: u32,
    ) -> Option<BleCameraAlert> {
        if ignore.contains(&entry.addr) {
            return None;
        }

        let st = self.per_src.entry(entry.addr).or_default();
        st.last_seen_ms = now_ms;

        if st.last_alert_ms != 0 && now_ms.saturating_sub(st.last_alert_ms) < PER_SRC_COOLDOWN_MS {
            return None;
        }

        if !ble_mask_needs_detection(disable_ble_mask) {
            return None;
        }

        let xuntong_enabled = !is_ble_method_disabled(
            disable_ble_mask,
            DetectionBleMethod::XuntongManufacturerId.as_u8(),
        );
        let name_keyword_enabled = !is_ble_method_disabled(
            disable_ble_mask,
            DetectionBleMethod::AdvertisedNameKeyword.as_u8(),
        );
        let infra_oui_enabled = !is_ble_method_disabled(
            disable_ble_mask,
            DetectionBleMethod::MacOuiInfrastructure.as_u8(),
        );
        let axon_enabled =
            !is_ble_method_disabled(disable_ble_mask, DetectionBleMethod::AxonOui.as_u8());
        let ipro_enabled =
            !is_ble_method_disabled(disable_ble_mask, DetectionBleMethod::IproOui.as_u8());

        // Priority: Xuntong company ID → name keywords → LPR-camera infra OUI → Axon OUI → i-PRO OUI.
        let method_id = if xuntong_enabled && entry.company_id == COMPANY_ID_XUNTONG {
            DetectionBleMethod::XuntongManufacturerId.as_u8()
        } else if name_keyword_enabled && name_matches_lpr_camera_keywords(&entry.name) {
            DetectionBleMethod::AdvertisedNameKeyword.as_u8()
        } else if infra_oui_enabled && ble_addr_matches_lpr_camera_oui(&entry.addr) {
            DetectionBleMethod::MacOuiInfrastructure.as_u8()
        } else if axon_enabled && is_axon_oui(&entry.addr) {
            DetectionBleMethod::AxonOui.as_u8()
        } else if ipro_enabled && is_ipro_oui(&entry.addr) {
            DetectionBleMethod::IproOui.as_u8()
        } else {
            return None;
        };

        st.last_alert_ms = now_ms;
        Some(BleCameraAlert {
            sniffer_id,
            src: entry.addr,
            rssi: entry.rssi,
            company_id: entry.company_id,
            method: method_id,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BleCameraAlert {
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub rssi: i8,
    pub company_id: u16,
    /// LPR-camera BLE **32..=34** or mobile surveillance Axon (**35**) / i-PRO (**36**).
    pub method: u8,
}

fn name_bytes_trimmed(name: &[u8; 22]) -> &[u8] {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    &name[..end]
}

fn name_matches_lpr_camera_keywords(name: &[u8; 22]) -> bool {
    let hay = name_bytes_trimmed(name);
    if hay.is_empty() {
        return false;
    }
    const NEEDLES: &[&[u8]] = &[b"flock", b"penguin", b"pigvision", b"fs ext battery"];
    for n in NEEDLES {
        if ascii_ci_contains(hay, n) {
            return true;
        }
    }
    false
}

fn ascii_ci_contains(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    'outer: for i in 0..=hay.len() - needle.len() {
        for (j, nb) in needle.iter().enumerate() {
            let a = hay[i + j];
            let b = *nb;
            let al = if a.is_ascii_uppercase() {
                a.to_ascii_lowercase()
            } else {
                a
            };
            let bl = if b.is_ascii_uppercase() {
                b.to_ascii_lowercase()
            } else {
                b
            };
            if al != bl {
                continue 'outer;
            }
        }
        return true;
    }
    false
}
