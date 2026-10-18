//! OpenDroneID / ASTM F3411 decoded UAS data structures.

pub const MESSAGE_SIZE: usize = 25;
pub const ID_SIZE: usize = 20;
pub const BASIC_ID_MAX_MESSAGES: usize = 2;

/// Encoded OpenDroneID message type (high nibble of byte 0).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteIdMessageType {
    BasicId = 0,
    Location = 1,
    Auth = 2,
    SelfId = 3,
    System = 4,
    OperatorId = 5,
    Packed = 0x0F,
    Invalid = 0xFF,
}

impl RemoteIdMessageType {
    #[must_use]
    pub const fn from_encoded_byte(byte: u8) -> Self {
        match byte >> 4 {
            0 => Self::BasicId,
            1 => Self::Location,
            2 => Self::Auth,
            3 => Self::SelfId,
            4 => Self::System,
            5 => Self::OperatorId,
            0x0F => Self::Packed,
            _ => Self::Invalid,
        }
    }
}

/// Wi-Fi transport that carried a Remote ID payload.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteIdWifiTransport {
    Beacon = 0,
    Nan = 1,
}

/// Decoded Basic ID message.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RemoteIdBasicId {
    pub ua_type: u8,
    pub id_type: u8,
    pub uas_id: [u8; ID_SIZE],
}

/// Decoded Location / Vector message.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RemoteIdLocation {
    pub status: u8,
    pub direction_deg: f32,
    pub speed_horizontal_mps: f32,
    pub speed_vertical_mps: f32,
    pub latitude: f64,
    pub longitude: f64,
    pub altitude_baro_m: f32,
    pub altitude_geo_m: f32,
    pub height_type: u8,
    pub height_m: f32,
    pub horiz_accuracy: u8,
    pub vert_accuracy: u8,
    pub baro_accuracy: u8,
    pub speed_accuracy: u8,
    pub ts_accuracy: u8,
    pub timestamp_sec: f32,
}

/// Decoded System message.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RemoteIdSystem {
    pub operator_location_type: u8,
    pub classification_type: u8,
    pub operator_latitude: f64,
    pub operator_longitude: f64,
    pub area_count: u16,
    pub area_radius_m: u16,
    pub area_ceiling_m: f32,
    pub area_floor_m: f32,
    pub category_eu: u8,
    pub class_eu: u8,
    pub operator_altitude_geo_m: f32,
    pub timestamp: u32,
}

/// Decoded Operator ID message.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct RemoteIdOperatorId {
    pub operator_id_type: u8,
    pub operator_id: [u8; ID_SIZE],
}

/// Aggregated UAS data updated by successive OpenDroneID decodes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteIdUasData {
    pub basic_id: [RemoteIdBasicId; BASIC_ID_MAX_MESSAGES],
    pub basic_id_valid: [bool; BASIC_ID_MAX_MESSAGES],
    pub location: RemoteIdLocation,
    pub location_valid: bool,
    pub system: RemoteIdSystem,
    pub system_valid: bool,
    pub operator_id: RemoteIdOperatorId,
    pub operator_id_valid: bool,
}

impl RemoteIdUasData {
    #[must_use]
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// True when decoded data is complete enough to emit an operator alert.
    #[must_use]
    pub fn is_alert_ready(&self) -> bool {
        if !self.location_valid {
            return false;
        }
        self.basic_id_valid.iter().any(|&v| v) || self.system_valid
    }

    /// Primary UAS ID bytes when any Basic ID slot is valid.
    #[must_use]
    pub fn primary_uas_id(&self) -> Option<&[u8; ID_SIZE]> {
        for (valid, slot) in self.basic_id_valid.iter().zip(self.basic_id.iter()) {
            if *valid {
                return Some(&slot.uas_id);
            }
        }
        None
    }
}

/// Decode packed latitude / longitude (`int32` wire value → degrees).
#[must_use]
pub const fn decode_lat_lon(encoded: i32) -> f64 {
    encoded as f64 / 10_000_000.0
}

/// True when decoded lat/lon are within normative ranges.
#[must_use]
pub fn lat_lon_valid(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)
}

/// Decode packed altitude / height (`uint16` wire value → meters).
#[must_use]
pub fn decode_altitude(encoded: u16) -> f32 {
    encoded as f32 * 0.5 - 1000.0
}

/// Decode packed horizontal speed.
#[must_use]
pub fn decode_speed_horizontal(encoded: u8, speed_mult: bool) -> f32 {
    if speed_mult {
        f32::from(encoded) * 0.75 + 63.75
    } else {
        f32::from(encoded) * 0.25
    }
}

/// Decode packed vertical speed.
#[must_use]
pub fn decode_speed_vertical(encoded: i8) -> f32 {
    f32::from(encoded) * 0.5
}

/// Decode packed direction (degrees, 0–359).
#[must_use]
pub fn decode_direction(encoded: u8, ew_direction: bool) -> f32 {
    if ew_direction {
        f32::from(encoded) + 180.0
    } else {
        f32::from(encoded)
    }
}

/// Decode packed area radius (meters).
#[must_use]
pub fn decode_area_radius(encoded: u8) -> u16 {
    u16::from(encoded) * 10
}

/// Decode location timestamp (seconds since the hour; `0xFFFF` = invalid).
#[must_use]
pub fn decode_timestamp(encoded: u16) -> f32 {
    if encoded == 0xFFFF {
        0xFFFF as f32
    } else {
        f32::from(encoded) / 10.0
    }
}

/// Copy a fixed-size ASCII field from wire bytes (null-padded).
#[must_use]
pub fn copy_ascii_field(src: &[u8], dst: &mut [u8]) {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    for b in &mut dst[n..] {
        *b = 0;
    }
}
