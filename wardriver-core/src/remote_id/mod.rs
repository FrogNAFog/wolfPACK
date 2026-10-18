//! ASTM F3411 / OpenDroneID passive detection and decode.

pub mod ble_extract;
pub mod decode;
pub mod detector;
pub mod types;
pub mod wifi_extract;

pub use ble_extract::extract_ble_remote_id_payload;
pub use decode::{decode_message, decode_open_drone_id_payload};
pub use detector::{RemoteIdAlert, RemoteIdDetector, PER_SRC_COOLDOWN_MS};
pub use types::{
    RemoteIdBasicId, RemoteIdLocation, RemoteIdMessageType, RemoteIdOperatorId, RemoteIdSystem,
    RemoteIdUasData, RemoteIdWifiTransport,
};
pub use wifi_extract::extract_wifi_remote_id_from_mgmt;

/// True when `ch` is a normative Remote ID Wi-Fi channel (2.4 GHz ch 6 or 5 GHz ch 149).
#[inline]
#[must_use]
pub const fn is_remote_id_wifi_channel(ch: u8) -> bool {
    matches!(ch, 6 | 149)
}
