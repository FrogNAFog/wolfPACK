//! OpenDroneID message decode (ASTM F3411 / opendroneid-core-c layouts).

use crate::remote_id::types::{
    copy_ascii_field, decode_altitude, decode_area_radius, decode_direction, decode_lat_lon,
    decode_speed_horizontal, decode_speed_vertical, decode_timestamp, RemoteIdBasicId,
    RemoteIdLocation, RemoteIdMessageType, RemoteIdOperatorId, RemoteIdSystem, RemoteIdUasData,
    BASIC_ID_MAX_MESSAGES, ID_SIZE, MESSAGE_SIZE,
};

const PACK_MAX_MESSAGES: usize = 9;

/// Decode one 25-byte OpenDroneID message into `uas`.
#[must_use]
pub fn decode_message(bytes: &[u8], uas: &mut RemoteIdUasData) -> Option<RemoteIdMessageType> {
    if bytes.len() < MESSAGE_SIZE {
        return None;
    }
    let msg = &bytes[..MESSAGE_SIZE];
    match RemoteIdMessageType::from_encoded_byte(msg[0]) {
        RemoteIdMessageType::BasicId => {
            decode_basic_id(msg, uas).then_some(RemoteIdMessageType::BasicId)
        }
        RemoteIdMessageType::Location => {
            decode_location(msg, uas).then_some(RemoteIdMessageType::Location)
        }
        RemoteIdMessageType::System => {
            decode_system(msg, uas).then_some(RemoteIdMessageType::System)
        }
        RemoteIdMessageType::OperatorId => {
            decode_operator_id(msg, uas).then_some(RemoteIdMessageType::OperatorId)
        }
        _ => None,
    }
}

/// Decode a transport payload: optional rotation counter, single message, or message pack (`0xF`).
#[must_use]
pub fn decode_open_drone_id_payload(
    payload: &[u8],
    uas: &mut RemoteIdUasData,
) -> Option<RemoteIdMessageType> {
    if payload.is_empty() {
        return None;
    }

    let rest = strip_rotation_counter(payload);

    if RemoteIdMessageType::from_encoded_byte(rest[0]) == RemoteIdMessageType::Packed {
        return decode_message_pack(rest, uas);
    }
    if rest.len() >= MESSAGE_SIZE {
        return decode_message(&rest[..MESSAGE_SIZE], uas);
    }
    None
}

/// Skip a leading 1-byte BLE / Wi-Fi rotation counter when present.
fn strip_rotation_counter(payload: &[u8]) -> &[u8] {
    if payload.len() == MESSAGE_SIZE + 1 {
        return &payload[1..];
    }
    if payload.len() > MESSAGE_SIZE
        && RemoteIdMessageType::from_encoded_byte(payload[0]) != RemoteIdMessageType::Packed
        && is_plausible_message_header(payload[1])
    {
        return &payload[1..];
    }
    payload
}

fn is_plausible_message_header(byte: u8) -> bool {
    matches!(
        RemoteIdMessageType::from_encoded_byte(byte),
        RemoteIdMessageType::BasicId
            | RemoteIdMessageType::Location
            | RemoteIdMessageType::System
            | RemoteIdMessageType::OperatorId
            | RemoteIdMessageType::Packed
    )
}

fn decode_message_pack(pack: &[u8], uas: &mut RemoteIdUasData) -> Option<RemoteIdMessageType> {
    if pack.len() < 3 {
        return None;
    }
    if RemoteIdMessageType::from_encoded_byte(pack[0]) != RemoteIdMessageType::Packed {
        return None;
    }
    let single_size = pack[1] as usize;
    let msg_count = pack[2] as usize;
    if single_size != MESSAGE_SIZE || msg_count == 0 || msg_count > PACK_MAX_MESSAGES {
        return None;
    }
    let needed = 3 + msg_count * single_size;
    if pack.len() < needed {
        return None;
    }
    if !check_pack_content(&pack[3..needed], msg_count) {
        return None;
    }
    let mut last = RemoteIdMessageType::Packed;
    for i in 0..msg_count {
        let off = 3 + i * single_size;
        if let Some(ty) = decode_message(&pack[off..off + single_size], uas) {
            last = ty;
        }
    }
    Some(last)
}

fn check_pack_content(msgs: &[u8], amount: usize) -> bool {
    let mut counts = [0u8; 6];
    for i in 0..amount {
        let off = i * MESSAGE_SIZE;
        if off + MESSAGE_SIZE > msgs.len() {
            return false;
        }
        let ty = RemoteIdMessageType::from_encoded_byte(msgs[off]);
        let idx = match ty {
            RemoteIdMessageType::BasicId => 0,
            RemoteIdMessageType::Location => 1,
            RemoteIdMessageType::Auth => 2,
            RemoteIdMessageType::SelfId => 3,
            RemoteIdMessageType::System => 4,
            RemoteIdMessageType::OperatorId => 5,
            _ => return false,
        };
        counts[idx] = counts[idx].saturating_add(1);
    }
    counts[0] <= BASIC_ID_MAX_MESSAGES as u8
        && counts[1] <= 1
        && counts[2] <= 16
        && counts[3] <= 1
        && counts[4] <= 1
        && counts[5] <= 1
}

fn decode_basic_id(msg: &[u8], uas: &mut RemoteIdUasData) -> bool {
    if RemoteIdMessageType::from_encoded_byte(msg[0]) != RemoteIdMessageType::BasicId {
        return false;
    }
    let id_type = msg[1] >> 4;
    let ua_type = msg[1] & 0x0F;
    if id_type > 15 || ua_type > 15 {
        return false;
    }

    let mut decoded = RemoteIdBasicId {
        ua_type,
        id_type,
        uas_id: [0; ID_SIZE],
    };
    let _ = copy_ascii_field(&msg[2..22], &mut decoded.uas_id);

    for slot in 0..BASIC_ID_MAX_MESSAGES {
        let stored = uas.basic_id[slot].id_type;
        if stored == 0 || stored == id_type {
            uas.basic_id[slot] = decoded;
            uas.basic_id_valid[slot] = true;
            return true;
        }
    }
    false
}

fn decode_location(msg: &[u8], uas: &mut RemoteIdUasData) -> bool {
    if RemoteIdMessageType::from_encoded_byte(msg[0]) != RemoteIdMessageType::Location {
        return false;
    }
    let flags = msg[1];
    let status = flags >> 4;
    if status > 15 {
        return false;
    }
    let speed_mult = (flags & 0x01) != 0;
    let ew_direction = (flags & 0x02) != 0;
    let height_type = (flags & 0x04) != 0;

    let lat = decode_lat_lon(i32::from_le_bytes(msg[5..9].try_into().unwrap_or([0; 4])));
    let lon = decode_lat_lon(i32::from_le_bytes(msg[9..13].try_into().unwrap_or([0; 4])));

    uas.location = RemoteIdLocation {
        status,
        direction_deg: decode_direction(msg[2], ew_direction),
        speed_horizontal_mps: decode_speed_horizontal(msg[3], speed_mult),
        speed_vertical_mps: decode_speed_vertical(msg[4] as i8),
        latitude: lat,
        longitude: lon,
        altitude_baro_m: decode_altitude(u16::from_le_bytes(
            msg[13..15].try_into().unwrap_or([0; 2]),
        )),
        altitude_geo_m: decode_altitude(u16::from_le_bytes(
            msg[15..17].try_into().unwrap_or([0; 2]),
        )),
        height_type: u8::from(height_type),
        height_m: decode_altitude(u16::from_le_bytes(msg[17..19].try_into().unwrap_or([0; 2]))),
        horiz_accuracy: msg[19] & 0x0F,
        vert_accuracy: msg[19] >> 4,
        speed_accuracy: msg[20] & 0x0F,
        baro_accuracy: msg[20] >> 4,
        ts_accuracy: msg[23] & 0x0F,
        timestamp_sec: decode_timestamp(u16::from_le_bytes(
            msg[21..23].try_into().unwrap_or([0; 2]),
        )),
    };
    uas.location_valid = true;
    true
}

fn decode_system(msg: &[u8], uas: &mut RemoteIdUasData) -> bool {
    if RemoteIdMessageType::from_encoded_byte(msg[0]) != RemoteIdMessageType::System {
        return false;
    }
    let flags = msg[1];
    uas.system = RemoteIdSystem {
        operator_location_type: flags & 0x03,
        classification_type: (flags >> 2) & 0x07,
        operator_latitude: decode_lat_lon(i32::from_le_bytes(
            msg[2..6].try_into().unwrap_or([0; 4]),
        )),
        operator_longitude: decode_lat_lon(i32::from_le_bytes(
            msg[6..10].try_into().unwrap_or([0; 4]),
        )),
        area_count: u16::from_le_bytes(msg[10..12].try_into().unwrap_or([0; 2])),
        area_radius_m: decode_area_radius(msg[12]),
        area_ceiling_m: decode_altitude(u16::from_le_bytes(
            msg[13..15].try_into().unwrap_or([0; 2]),
        )),
        area_floor_m: decode_altitude(u16::from_le_bytes(msg[15..17].try_into().unwrap_or([0; 2]))),
        category_eu: msg[17] >> 4,
        class_eu: msg[17] & 0x0F,
        operator_altitude_geo_m: decode_altitude(u16::from_le_bytes(
            msg[18..20].try_into().unwrap_or([0; 2]),
        )),
        timestamp: u32::from_le_bytes(msg[20..24].try_into().unwrap_or([0; 4])),
    };
    uas.system_valid = true;
    true
}

fn decode_operator_id(msg: &[u8], uas: &mut RemoteIdUasData) -> bool {
    if RemoteIdMessageType::from_encoded_byte(msg[0]) != RemoteIdMessageType::OperatorId {
        return false;
    }
    let mut operator_id = [0u8; ID_SIZE];
    let _ = copy_ascii_field(&msg[2..22], &mut operator_id);
    uas.operator_id = RemoteIdOperatorId {
        operator_id_type: msg[1],
        operator_id,
    };
    uas.operator_id_valid = true;
    true
}
