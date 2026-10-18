//! Alert taxonomy shared by stats counters and control feedback (buzzer/OLED).

/// NVS bitmask: bits 0..7 set => sound enabled for user-configurable [`AlertKind`] events.
pub const ALERT_SOUND_MASK_ALL: u8 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AlertKind {
    Boot = 0,
    UploadStart = 1,
    UploadDone = 2,
    GpsFix = 3,
    Camera = 4,
    Surveillance = 5,
    /// Beacon SSID watch hit (operator-defined list on `/detections`).
    SsidWatch = 6,
    /// SD mount failure (buzzer only; not in NVS mask / web UI).
    SdFail = 7,
    /// Waiting for a usable GPS fix (OLED only; module talking, no fix yet).
    GpsWaiting = 8,
    /// No UART/NMEA from GNSS (OLED only; wiring/module not responding).
    GpsNoModule = 9,
    /// Fleet roster cleared via BOOT long-press (OLED + buzzer).
    FleetUnbound = 10,
    /// OpenDroneID / ASTM F3411 UAS detection.
    RemoteId = 11,
}

impl AlertKind {
    pub const COUNT: usize = 8;

    pub fn sound_bit(self) -> u8 {
        match self {
            Self::RemoteId => 1 << 7,
            _ => 1u8 << (self as u8),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Boot => "BOOT",
            Self::UploadStart => "UPLOAD START",
            Self::UploadDone => "UPLOAD DONE",
            Self::GpsFix => "GPS FIX",
            Self::Camera => "CAMERA",
            Self::Surveillance => "AXON",
            Self::SsidWatch => "SSID WATCH",
            Self::SdFail => "SD FAIL",
            Self::GpsWaiting => "NO FIX",
            Self::GpsNoModule => "NO GPS",
            Self::FleetUnbound => "UNBOUND",
            Self::RemoteId => "REMOTE ID",
        }
    }

    /// User-configurable events (bits in NVS `alert_snd`).
    pub fn is_user_configurable(self) -> bool {
        matches!(
            self,
            Self::Boot
                | Self::UploadStart
                | Self::UploadDone
                | Self::GpsFix
                | Self::Camera
                | Self::Surveillance
                | Self::SsidWatch
                | Self::RemoteId
        )
    }

    pub fn is_persistent_status(self) -> bool {
        matches!(self, Self::GpsWaiting | Self::GpsNoModule)
    }

    pub fn plays_sound(self) -> bool {
        !matches!(self, Self::GpsWaiting | Self::GpsNoModule)
    }
}
