//! Detection method IDs and disable-mask helpers (LPR-camera vs mobile surveillance).

/// RF / stack domain for a detection alert (extensible for BLE).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionSignalKind {
    Wifi = 0,
    Ble = 1,
}

/// WiFi-specific LPR-camera / mobile-surveillance detection methods (additive IDs `1..=31`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionWifiMethod {
    WildcardProbesOui = 1,
    WildcardProbeIeSignatureOui = 2,
    WildcardProbeIeSignatureAnyMac = 3,
    /// IEEE OUI **00:25:DF** — Axon body cameras (Wi-Fi mobile surveillance).
    AxonOui = 4,
    /// IEEE OUI **D4:2D:C5** — i-PRO body cameras (Wi-Fi mobile surveillance).
    IproOui = 5,
}

/// Passive BLE LPR-camera advertisement detectors (`32..=34`; pack-deprecated off by default).
/// [`DetectionBleMethod::AxonOui`] / [`DetectionBleMethod::IproOui`] are **mobile
/// surveillance** (PACK product slice), not LPR-camera map/alert taxonomy.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionBleMethod {
    XuntongManufacturerId = 32,
    AdvertisedNameKeyword = 33,
    MacOuiInfrastructure = 34,
    /// IEEE OUI **00:25:DF** — mobile surveillance (PACK `DetectionBleMethod::AxonOui`).
    AxonOui = 35,
    /// IEEE OUI **D4:2D:C5** — i-PRO body cameras (BLE mobile surveillance).
    IproOui = 36,
}

impl DetectionSignalKind {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl DetectionWifiMethod {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn caps(self) -> MethodCaps {
        match self {
            Self::WildcardProbesOui => MethodCaps {
                requires_wildcard_probe: true,
                requires_ie_sig: false,
                is_ble: false,
            },
            Self::WildcardProbeIeSignatureOui | Self::WildcardProbeIeSignatureAnyMac => {
                MethodCaps {
                    requires_wildcard_probe: true,
                    requires_ie_sig: true,
                    is_ble: false,
                }
            }
            Self::AxonOui | Self::IproOui => MethodCaps {
                requires_wildcard_probe: false,
                requires_ie_sig: false,
                is_ble: false,
            },
        }
    }
}

impl DetectionBleMethod {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn caps(self) -> MethodCaps {
        MethodCaps {
            requires_wildcard_probe: false,
            requires_ie_sig: false,
            is_ble: true,
        }
    }
}

/// Parser / runtime work required by a detection method (used to gate sniffers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodCaps {
    pub requires_wildcard_probe: bool,
    pub requires_ie_sig: bool,
    pub is_ble: bool,
}

const WIFI_FLOCK_METHOD_IDS: &[u8] = &[
    DetectionWifiMethod::WildcardProbesOui.as_u8(),
    DetectionWifiMethod::WildcardProbeIeSignatureOui.as_u8(),
    DetectionWifiMethod::WildcardProbeIeSignatureAnyMac.as_u8(),
    DetectionWifiMethod::AxonOui.as_u8(),
    DetectionWifiMethod::IproOui.as_u8(),
];

const BLE_FLOCK_METHOD_IDS: &[u8] = &[
    DetectionBleMethod::XuntongManufacturerId.as_u8(),
    DetectionBleMethod::AdvertisedNameKeyword.as_u8(),
    DetectionBleMethod::MacOuiInfrastructure.as_u8(),
    DetectionBleMethod::AxonOui.as_u8(),
    DetectionBleMethod::IproOui.as_u8(),
];

/// Capability metadata for WiFi LPR-camera method `m` (`1..=31`); unknown IDs return all-false.
#[must_use]
pub const fn wifi_method_caps(method: u8) -> MethodCaps {
    match method {
        1 => DetectionWifiMethod::WildcardProbesOui.caps(),
        2 => DetectionWifiMethod::WildcardProbeIeSignatureOui.caps(),
        3 => DetectionWifiMethod::WildcardProbeIeSignatureAnyMac.caps(),
        4 => DetectionWifiMethod::AxonOui.caps(),
        5 => DetectionWifiMethod::IproOui.caps(),
        _ => MethodCaps {
            requires_wildcard_probe: false,
            requires_ie_sig: false,
            is_ble: false,
        },
    }
}

/// Capability metadata for BLE LPR-camera / mobile-surveillance method `m` (`32..=63`); unknown IDs return all-false.
#[must_use]
pub const fn ble_method_caps(method: u8) -> MethodCaps {
    match method {
        32 => DetectionBleMethod::XuntongManufacturerId.caps(),
        33 => DetectionBleMethod::AdvertisedNameKeyword.caps(),
        34 => DetectionBleMethod::MacOuiInfrastructure.caps(),
        35 => DetectionBleMethod::AxonOui.caps(),
        36 => DetectionBleMethod::IproOui.caps(),
        _ => MethodCaps {
            requires_wildcard_probe: false,
            requires_ie_sig: false,
            is_ble: false,
        },
    }
}

/// True when any **enabled** WiFi LPR-camera method needs wildcard probe parsing / `eval_burst`.
#[inline]
#[must_use]
pub fn wifi_mask_needs_wildcard_probe(disable_mask: u32) -> bool {
    WIFI_FLOCK_METHOD_IDS.iter().any(|&m| {
        !is_wifi_method_disabled(disable_mask, m) && wifi_method_caps(m).requires_wildcard_probe
    })
}

/// True when any **enabled** WiFi LPR-camera method needs the primary IE TLV matcher.
#[inline]
#[must_use]
pub fn wifi_mask_needs_ie_sig(disable_mask: u32) -> bool {
    WIFI_FLOCK_METHOD_IDS
        .iter()
        .any(|&m| !is_wifi_method_disabled(disable_mask, m) && wifi_method_caps(m).requires_ie_sig)
}

/// True when any **enabled** Wi-Fi mobile-surveillance OUI method (4 or 5) may fire.
#[inline]
#[must_use]
pub fn wifi_mask_needs_ms_oui(disable_mask: u32) -> bool {
    !is_wifi_method_disabled(disable_mask, DetectionWifiMethod::AxonOui.as_u8())
        || !is_wifi_method_disabled(disable_mask, DetectionWifiMethod::IproOui.as_u8())
}

/// True when any **enabled** BLE detection method (32–36) may fire alerts.
#[inline]
#[must_use]
pub fn ble_mask_needs_detection(disable_mask: u32) -> bool {
    BLE_FLOCK_METHOD_IDS
        .iter()
        .any(|&m| !is_ble_method_disabled(disable_mask, m))
}

/// PACK [`map_data::MOBILE_SURVEILLANCE_AXON_METHOD_ID`]: BLE Axon OUI rows and map pins.
pub const MOBILE_SURVEILLANCE_AXON_METHOD_ID: u8 = DetectionBleMethod::AxonOui.as_u8();

/// Wi-Fi Axon OUI method ID (**4**).
pub const MOBILE_SURVEILLANCE_AXON_WIFI_METHOD_ID: u8 = DetectionWifiMethod::AxonOui.as_u8();

/// Wi-Fi i-PRO OUI method ID (**5**).
pub const MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID: u8 = DetectionWifiMethod::IproOui.as_u8();

/// BLE i-PRO OUI method ID (**36**).
pub const MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID: u8 = DetectionBleMethod::IproOui.as_u8();

/// True when `method` is any mobile-surveillance detector (Wi-Fi **4/5** or BLE **35/36**).
#[inline]
#[must_use]
pub fn is_mobile_surveillance_method(method: u8) -> bool {
    matches!(
        method,
        MOBILE_SURVEILLANCE_AXON_WIFI_METHOD_ID
            | MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID
            | MOBILE_SURVEILLANCE_AXON_METHOD_ID
            | MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID
    )
}

/// Returns true when `method` is a mobile-surveillance BLE detector (not LPR-camera infrastructure BLE).
#[inline]
#[must_use]
pub fn is_mobile_surveillance_ble_method(method: u8) -> bool {
    method == MOBILE_SURVEILLANCE_AXON_METHOD_ID || method == MOBILE_SURVEILLANCE_IPRO_BLE_METHOD_ID
}

/// Returns true when `method` is a mobile-surveillance Wi-Fi detector.
#[inline]
#[must_use]
pub fn is_mobile_surveillance_wifi_method(method: u8) -> bool {
    method == MOBILE_SURVEILLANCE_AXON_WIFI_METHOD_ID
        || method == MOBILE_SURVEILLANCE_IPRO_WIFI_METHOD_ID
}

/// Bit `m-1` of the WiFi disable mask disables WiFi method `m` (`1..=31`).
#[inline]
#[must_use]
pub const fn detection_wifi_disable_bit(method: u8) -> u32 {
    if method == 0 || method > 31 {
        0
    } else {
        1u32 << (method - 1)
    }
}

/// Bit `m-32` of the LPR-camera BLE disable mask disables BLE method `m` (`32..=63`), including Axon (**35**).
#[inline]
#[must_use]
pub const fn detection_ble_disable_bit(method: u8) -> u32 {
    if method >= 32 && method <= 63 {
        1u32 << (method - 32)
    } else {
        0
    }
}

#[inline]
#[must_use]
pub fn is_wifi_method_disabled(mask: u32, method: u8) -> bool {
    mask & detection_wifi_disable_bit(method) != 0
}

#[inline]
#[must_use]
pub fn is_ble_method_disabled(mask: u32, method: u8) -> bool {
    mask & detection_ble_disable_bit(method) != 0
}

/// Default `detection_disable_wifi_mask` (deprecated WiFi method 1 off; Axon/i-PRO Wi-Fi on).
pub const DETECTION_WIFI_DISABLE_MASK_DEFAULT: u32 = detection_wifi_disable_bit(1);

/// Default `detection_disable_ble_mask` (LPR-camera BLE methods 32–34 off; Axon **35** / i-PRO **36** on).
pub const DETECTION_BLE_DISABLE_MASK_DEFAULT: u32 =
    detection_ble_disable_bit(32) | detection_ble_disable_bit(33) | detection_ble_disable_bit(34);

/// Wi-Fi Remote ID detection methods (`64..=95`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteIdWifiDetectionMethod {
    BeaconVendorIe = 64,
    NanAction = 65,
}

/// BLE Remote ID detection methods (`96..=127`).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteIdBleDetectionMethod {
    ServiceDataFffa = 96,
    LegacyMfg0002 = 97,
}

impl RemoteIdWifiDetectionMethod {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

impl RemoteIdBleDetectionMethod {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Bit `m-64` of the Remote ID Wi-Fi disable mask disables method `m` (`64..=95`).
#[inline]
#[must_use]
pub const fn remote_id_wifi_disable_bit(method: u8) -> u32 {
    if method >= 64 && method <= 95 {
        1u32 << (method - 64)
    } else {
        0
    }
}

/// Bit `m-96` of the Remote ID BLE disable mask disables method `m` (`96..=127`).
#[inline]
#[must_use]
pub const fn remote_id_ble_disable_bit(method: u8) -> u32 {
    if method >= 96 && method <= 127 {
        1u32 << (method - 96)
    } else {
        0
    }
}

#[inline]
#[must_use]
pub fn is_remote_id_wifi_method_disabled(mask: u32, method: u8) -> bool {
    mask & remote_id_wifi_disable_bit(method) != 0
}

#[inline]
#[must_use]
pub fn is_remote_id_ble_method_disabled(mask: u32, method: u8) -> bool {
    mask & remote_id_ble_disable_bit(method) != 0
}
