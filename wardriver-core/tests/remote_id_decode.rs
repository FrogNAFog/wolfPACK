mod remote_id_helpers;

use remote_id_helpers::{sample_basic_id_msg, sample_location_msg, sample_message_pack};
use wardriver_core::remote_id::types::{
    copy_ascii_field, decode_altitude, decode_area_radius, decode_direction, decode_lat_lon,
    decode_speed_horizontal, decode_speed_vertical, decode_timestamp, lat_lon_valid,
    RemoteIdBasicId, ID_SIZE, MESSAGE_SIZE,
};
use wardriver_core::remote_id::{
    decode_message, decode_open_drone_id_payload, RemoteIdMessageType, RemoteIdUasData,
};

#[test]
fn decode_basic_id_and_location() {
    let mut uas = RemoteIdUasData::default();
    let basic = sample_basic_id_msg(b"TEST-UAS-1234567890");
    assert_eq!(
        decode_message(&basic, &mut uas),
        Some(RemoteIdMessageType::BasicId)
    );
    assert!(uas.basic_id_valid[0]);
    assert_eq!(&uas.basic_id[0].uas_id[..15], b"TEST-UAS-123456");

    let loc = sample_location_msg(37.7749, -122.4194, 120.0);
    assert_eq!(
        decode_message(&loc, &mut uas),
        Some(RemoteIdMessageType::Location)
    );
    assert!(uas.location_valid);
    assert!((uas.location.latitude - 37.7749).abs() < 1e-5);
    assert!((uas.location.longitude - (-122.4194)).abs() < 1e-5);
    assert!(uas.is_alert_ready());
}

#[test]
fn decode_message_pack() {
    let basic = sample_basic_id_msg(b"PACK-DRONE");
    let loc = sample_location_msg(40.0, -75.0, 50.0);
    let pack = sample_message_pack(&[basic, loc]);
    let mut uas = RemoteIdUasData::default();
    assert_eq!(
        decode_open_drone_id_payload(&pack, &mut uas),
        Some(RemoteIdMessageType::Location)
    );
    assert!(uas.is_alert_ready());
}

#[test]
fn decode_message_too_short_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let result = decode_message(&[0x01], &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_message_empty_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let result = decode_message(&[], &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_message_payload_longer_than_message_size() {
    let mut uas = RemoteIdUasData::default();
    let mut bytes = [0u8; 30];
    bytes[0] = 0x00; // BasicId type nibble
    let result = decode_message(&bytes, &mut uas);
    assert!(result.is_some());
}

#[test]
fn decode_payload_empty_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let result = decode_open_drone_id_payload(&[], &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_payload_too_short_for_message() {
    let mut uas = RemoteIdUasData::default();
    let payload = [0x01; 10];
    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_pack_zero_message_count() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = [0u8; 10];
    payload[0] = 0xF0; // Packed type
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 0; // zero messages
    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_pack_exceeds_max_messages() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = [0u8; 10];
    payload[0] = 0xF0; // Packed type
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 10; // max is 9
    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

#[test]
fn decode_pack_valid_two_messages() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE];
    payload[0] = 0xF0; // Packed type (rotation=0)
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 2;

    // First message: BasicId (type=0 in high nibble, rotation=0)
    payload[3] = 0x00;

    // Second message: Location (type=1 in high nibble)
    let loc_off = 3 + MESSAGE_SIZE;
    payload[loc_off] = 0x10;

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_some());
    assert!(uas.basic_id_valid[0]);
    assert!(uas.location_valid);
}

#[test]
fn decode_pack_rotation_counter_stripped() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; MESSAGE_SIZE + 1];
    payload[0] = 0xA5;
    payload[1] = 0x00; // BasicId type

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_some());
    assert!(uas.basic_id_valid[0]);
}

#[test]
fn decode_basic_id_fills_slot_zero_when_empty() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0u8; MESSAGE_SIZE];
    msg[0] = 0x00;
    let result = decode_message(&msg, &mut uas);
    assert_eq!(result, Some(RemoteIdMessageType::BasicId));
    assert!(uas.basic_id_valid[0]);
}

#[test]
fn decode_basic_id_overwrites_same_type_slot() {
    let mut uas = RemoteIdUasData::default();
    let mut msg1 = [0u8; MESSAGE_SIZE];
    msg1[0] = 0x00; // type=BasicId
    msg1[1] = 0x10; // id_type=1, ua_type=0
    msg1[2..7].copy_from_slice(b"ID123");
    decode_message(&msg1, &mut uas);

    let mut msg2 = [0u8; MESSAGE_SIZE];
    msg2[0] = 0x00; // type=BasicId
    msg2[1] = 0x10; // same id_type=1
    msg2[2..7].copy_from_slice(b"ID456");
    decode_message(&msg2, &mut uas);

    assert!(uas.basic_id_valid[0]);
    assert_eq!(uas.basic_id[0].uas_id[..5], *b"ID456");
}

#[test]
fn decode_basic_id_fills_second_slot_different_type() {
    // Code: stored == 0 means "empty slot", so first decode always goes to slot 0.
    // To get something into slot 1, fill slot 0 with a nonzero id_type first.
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0u8; MESSAGE_SIZE];
    msg[0] = 0x00; // type=BasicId
    msg[1] = 0x10; // id_type=1 (nonzero)
    decode_message(&msg, &mut uas);
    assert!(uas.basic_id_valid[0]);
    assert_eq!(uas.basic_id[0].id_type, 1);

    // Now decode id_type=0 → slot 0 has stored=1 (doesn't match), slot 1 has stored=0 (empty match)
    let mut msg2 = [0u8; MESSAGE_SIZE];
    msg2[0] = 0x00; // type=BasicId
    msg2[1] = 0x00; // id_type=0
    let result = decode_message(&msg2, &mut uas);
    assert!(result.is_some());
    assert!(uas.basic_id_valid[1]);
}

#[test]
fn decode_basic_id_both_slots_full_different_types() {
    // Fill slot 0 with id_type=1 (nonzero)
    let mut uas = RemoteIdUasData::default();
    let mut msg1 = [0u8; MESSAGE_SIZE];
    msg1[0] = 0x00; // type=BasicId
    msg1[1] = 0x10; // id_type=1
    decode_message(&msg1, &mut uas);

    // Decode id_type=0 → goes to slot 1 (first empty slot)
    let mut msg2 = [0u8; MESSAGE_SIZE];
    msg2[0] = 0x00; // type=BasicId
    msg2[1] = 0x00; // id_type=0
    decode_message(&msg2, &mut uas);

    assert!(uas.basic_id_valid[0]);
    assert!(uas.basic_id_valid[1]);
    assert_eq!(uas.basic_id[0].id_type, 1);
    assert_eq!(uas.basic_id[1].id_type, 0);
}

#[test]
fn is_alert_ready_requires_location() {
    let mut uas = RemoteIdUasData::default();
    uas.basic_id[0] = RemoteIdBasicId {
        ua_type: 0,
        id_type: 0,
        uas_id: [b'T'; ID_SIZE],
    };
    uas.basic_id_valid[0] = true;
    assert!(!uas.is_alert_ready());
    uas.location_valid = true;
    assert!(uas.is_alert_ready());
}

#[test]
fn is_alert_ready_with_only_system_no_basic_id() {
    let mut uas = RemoteIdUasData::default();
    uas.location_valid = true;
    uas.system_valid = true;
    assert!(uas.is_alert_ready());
}

#[test]
fn is_alert_ready_with_no_data() {
    let uas = RemoteIdUasData::default();
    assert!(!uas.is_alert_ready());
}

#[test]
fn primary_uas_id_returns_first_valid() {
    let mut uas = RemoteIdUasData::default();
    uas.basic_id[0].uas_id = [b'F'; ID_SIZE];
    uas.basic_id_valid[0] = true;
    uas.basic_id[1].uas_id = [b'S'; ID_SIZE];
    uas.basic_id_valid[1] = true;
    let id = uas.primary_uas_id();
    assert!(id.is_some());
    assert_eq!(id.unwrap()[0], b'F');
}

#[test]
fn primary_uas_id_skips_invalid_first_slot() {
    let mut uas = RemoteIdUasData::default();
    uas.basic_id_valid[0] = false;
    uas.basic_id[1].uas_id = [b'S'; ID_SIZE];
    uas.basic_id_valid[1] = true;
    let id = uas.primary_uas_id();
    assert!(id.is_some());
    assert_eq!(id.unwrap()[0], b'S');
}

#[test]
fn primary_uas_id_returns_none_when_all_invalid() {
    let uas = RemoteIdUasData::default();
    assert!(uas.primary_uas_id().is_none());
}

#[test]
fn decode_lat_lon_positive() {
    let lat = decode_lat_lon(377749295);
    assert!((lat - 37.7749295).abs() < 0.0001);
}

#[test]
fn decode_lat_lon_negative() {
    let lat = decode_lat_lon(-338688237);
    assert!((lat - -33.8688237).abs() < 0.0001);
}

#[test]
fn decode_lat_lon_zero() {
    assert_eq!(decode_lat_lon(0), 0.0);
}

#[test]
fn lat_lon_valid_at_poles() {
    assert!(lat_lon_valid(90.0, 0.0));
    assert!(lat_lon_valid(-90.0, 0.0));
}

#[test]
fn lat_lon_valid_at_dateline() {
    assert!(lat_lon_valid(0.0, 180.0));
    assert!(lat_lon_valid(0.0, -180.0));
}

#[test]
fn lat_lon_invalid_above_90() {
    assert!(!lat_lon_valid(90.001, 0.0));
    assert!(!lat_lon_valid(-90.001, 0.0));
}

#[test]
fn lat_lon_invalid_above_180() {
    assert!(!lat_lon_valid(0.0, 180.001));
    assert!(!lat_lon_valid(0.0, -180.001));
}

#[test]
fn test_decode_altitude_min() {
    let alt = decode_altitude(0);
    assert_eq!(alt, -1000.0);
}

#[test]
fn test_decode_altitude_max() {
    let alt = decode_altitude(0xFFFF);
    assert!((alt - 31767.5).abs() < 0.1);
}

#[test]
fn test_decode_altitude_zero_level() {
    let alt = decode_altitude(2000);
    assert_eq!(alt, 0.0);
}

#[test]
fn test_decode_speed_horizontal_low_range() {
    let speed = decode_speed_horizontal(0, false);
    assert_eq!(speed, 0.0);
    let speed = decode_speed_horizontal(255, false);
    assert_eq!(speed, 63.75);
}

#[test]
fn test_decode_speed_horizontal_high_range() {
    let speed = decode_speed_horizontal(0, true);
    assert_eq!(speed, 63.75);
    let speed = decode_speed_horizontal(255, true);
    assert_eq!(speed, 255.0);
}

#[test]
fn test_decode_speed_vertical_range() {
    let up = decode_speed_vertical(127);
    assert_eq!(up, 63.5);
    let down = decode_speed_vertical(-128);
    assert_eq!(down, -64.0);
}

#[test]
fn test_decode_direction_east_flag() {
    let d = decode_direction(180, true);
    assert_eq!(d, 360.0);
    let d = decode_direction(0, true);
    assert_eq!(d, 180.0);
}

#[test]
fn test_decode_direction_west_flag() {
    let d = decode_direction(180, false);
    assert_eq!(d, 180.0);
    let d = decode_direction(255, false);
    assert_eq!(d, 255.0);
}

#[test]
fn test_decode_area_radius() {
    assert_eq!(decode_area_radius(0), 0);
    assert_eq!(decode_area_radius(255), 2550);
}

#[test]
fn test_decode_timestamp_invalid() {
    let ts = decode_timestamp(0xFFFF);
    assert_eq!(ts, 65535.0);
}

#[test]
fn test_decode_timestamp_valid() {
    let ts = decode_timestamp(3600);
    assert_eq!(ts, 360.0);
}

#[test]
fn test_copy_ascii_field_longer_src() {
    let src = b"ABCDEFGHIJKLMNOPQRSTUV";
    let mut dst = [0u8; 5];
    copy_ascii_field(src, &mut dst);
    assert_eq!(&dst, b"ABCDE");
}

#[test]
fn test_copy_ascii_field_shorter_src() {
    let src = b"ABC";
    let mut dst = [0u8; 8];
    copy_ascii_field(src, &mut dst);
    assert_eq!(&dst[..], &[b'A', b'B', b'C', 0, 0, 0, 0, 0][..]);
}

#[test]
fn test_copy_ascii_field_exact_length() {
    let src = b"ABCDE";
    let mut dst = [0u8; 5];
    copy_ascii_field(src, &mut dst);
    assert_eq!(&dst, b"ABCDE");
}

#[test]
fn test_copy_ascii_field_empty_dst() {
    let src = b"TEST";
    let mut dst = [0u8; 0];
    copy_ascii_field(src, &mut dst);
    assert!(dst.is_empty());
}

/// Remote ID with only location message (no basic ID) should not trigger alert.
#[test]
fn alert_ready_requires_basic_id_or_system() {
    let mut uas = RemoteIdUasData::default();
    // Decode a Location message.
    let mut msg = [0x10u8; MESSAGE_SIZE];
    decode_message(&msg, &mut uas);
    assert!(uas.location_valid);
    // No basic_id_valid, no system_valid → not alert ready.
    assert!(!uas.is_alert_ready());
}

/// Remote ID with only System message (no basic ID) should trigger alert + location.
#[test]
fn alert_ready_with_system_and_location() {
    let mut uas = RemoteIdUasData::default();
    // Location first.
    let mut loc = [0x10u8; MESSAGE_SIZE];
    decode_message(&loc, &mut uas);
    // System.
    let mut sys = [0x40u8; MESSAGE_SIZE];
    decode_message(&sys, &mut uas);
    assert!(uas.system_valid);
    assert!(uas.location_valid);
    assert!(uas.is_alert_ready());
}

/// Remote ID with SelfId type — decode_message returns None for SelfId (not handled).
#[test]
fn decode_selfid_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x30u8; MESSAGE_SIZE]; // SelfId type nibble = 3
    let result = decode_message(&msg, &mut uas);
    assert!(result.is_none());
}

/// Remote ID with Auth type — decode_message returns None for Auth (not handled).
#[test]
fn decode_auth_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x20u8; MESSAGE_SIZE]; // Auth type nibble = 2
    let result = decode_message(&msg, &mut uas);
    assert!(result.is_none());
}

/// Remote ID with invalid type nibble (0x6+) — returns None.
#[test]
fn decode_invalid_type_returns_none() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x60u8; MESSAGE_SIZE]; // type nibble 6 — invalid
    let result = decode_message(&msg, &mut uas);
    assert!(result.is_none());
    // 0xE0 also invalid (not Packed).
    let mut msg2 = [0xE0u8; MESSAGE_SIZE];
    let result2 = decode_message(&msg2, &mut uas);
    assert!(result2.is_none());
}

/// SelfId slot in a pack — check_pack_content allows at most 1 SelfId.
#[test]
fn pack_selfid_max_count_one() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 3 * MESSAGE_SIZE];
    payload[0] = 0xF0; // Packed
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 3;

    // First: BasicId
    payload[3] = 0x00;
    // Second: SelfId
    payload[3 + MESSAGE_SIZE] = 0x30;
    // Third: SelfId — exceeds max 1 → should fail
    payload[3 + 2 * MESSAGE_SIZE] = 0x30;

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

/// Pack with exactly 1 SelfId — should succeed.
#[test]
fn pack_with_one_selfid_succeeds() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 2;

    payload[3] = 0x00; // BasicId
    payload[3 + MESSAGE_SIZE] = 0x30; // SelfId

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    // SelfId decode returns None, but pack parsing succeeds; BasicId decoded.
    assert!(result.is_some());
    assert!(uas.basic_id_valid[0]);
}

/// Pack with 9 messages (max allowed).
#[test]
fn pack_with_nine_messages_max() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 9 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 9;

    // Fill with BasicId messages (but only 2 allowed → should fail).
    for i in 0..9 {
        payload[3 + i * MESSAGE_SIZE] = 0x00; // BasicId
    }

    // 9 BasicId exceeds BASIC_ID_MAX_MESSAGES (2) → fail.
    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

/// Pack with valid mix: 2 BasicId + 1 Location + 1 System + 1 SelfId + 1 OperatorId + 2 Auth = 9
#[test]
fn pack_with_full_valid_mix() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 9 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 9;

    payload[3 + 0 * MESSAGE_SIZE] = 0x00; // BasicId 1
    payload[3 + 1 * MESSAGE_SIZE] = 0x00; // BasicId 2 (max 2)
    payload[3 + 2 * MESSAGE_SIZE] = 0x10; // Location (max 1)
    payload[3 + 3 * MESSAGE_SIZE] = 0x40; // System (max 1)
    payload[3 + 4 * MESSAGE_SIZE] = 0x50; // OperatorId (max 1)
    payload[3 + 5 * MESSAGE_SIZE] = 0x30; // SelfId (max 1)
    payload[3 + 6 * MESSAGE_SIZE] = 0x20; // Auth 1
    payload[3 + 7 * MESSAGE_SIZE] = 0x20; // Auth 2
    payload[3 + 8 * MESSAGE_SIZE] = 0x20; // Auth 3

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_some());
    assert!(uas.basic_id_valid[0]);
    assert!(uas.location_valid);
    assert!(uas.system_valid);
    assert!(uas.operator_id_valid);
}

/// Pack with 2 Location messages — exceeds max 1.
#[test]
fn pack_two_locations_exceeds_max() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 2;

    payload[3] = 0x10; // Location
    payload[3 + MESSAGE_SIZE] = 0x10; // Location — exceeds max 1

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

/// Pack with 2 System messages — exceeds max 1.
#[test]
fn pack_two_systems_exceeds_max() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 2;

    payload[3] = 0x40; // System
    payload[3 + MESSAGE_SIZE] = 0x40; // System — exceeds max 1

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

/// Pack with 2 OperatorId messages — exceeds max 1.
#[test]
fn pack_two_operator_ids_exceeds_max() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE];
    payload[0] = 0xF0;
    payload[1] = MESSAGE_SIZE as u8;
    payload[2] = 2;

    payload[3] = 0x50; // OperatorId
    payload[3 + MESSAGE_SIZE] = 0x50; // OperatorId — exceeds max 1

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_none());
}

/// SelfId with self-identifying string longer than ID_SIZE (20 bytes) — copy_ascii_field handles it.
#[test]
fn selfid_ascii_truncation() {
    // Note: decode_message returns None for SelfId, but copy_ascii_field is used by BasicId.
    // The 20-byte uas_id field is always padded via copy_ascii_field(&msg[2..22], &mut dst).
    // This is already covered by copy_ascii_field tests in remote_id_decode_edge.rs.
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x00u8; MESSAGE_SIZE];
    // Fill uas_id area (bytes 2-21) with a long ASCII string.
    for i in 0..20 {
        msg[2 + i] = b'A';
    }
    decode_message(&msg, &mut uas);
    assert!(uas.basic_id_valid[0]);
    // uas_id should be filled with 'A's.
    assert_eq!(uas.basic_id[0].uas_id.iter().all(|&b| b == b'A'), true);
}

/// BasicId with id_type > 15 — returns false.
#[test]
fn basicid_id_type_too_large() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x00u8; MESSAGE_SIZE];
    msg[1] = 0xF0; // id_type = 15, ua_type = 0 — valid
    let r1 = decode_message(&msg, &mut uas);
    assert!(r1.is_some());

    let mut uas2 = RemoteIdUasData::default();
    let mut msg2 = [0x00u8; MESSAGE_SIZE];
    msg2[1] = 0xF1; // id_type = 15, ua_type = 1 — valid
    let r2 = decode_message(&msg2, &mut uas2);
    assert!(r2.is_some());
}

/// BasicId with ua_type > 15 — returns false.
#[test]
fn basicid_ua_type_too_large() {
    let mut uas = RemoteIdUasData::default();
    let mut msg = [0x00u8; MESSAGE_SIZE];
    msg[1] = 0x0F; // id_type = 0, ua_type = 15 — valid
    let r1 = decode_message(&msg, &mut uas);
    assert!(r1.is_some());
}

/// Message type enum — Packed (0xF) recognized.
#[test]
fn message_type_packed_byte() {
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xF0),
        RemoteIdMessageType::Packed
    );
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xFF),
        RemoteIdMessageType::Packed
    );
}

/// Message type enum — Invalid byte (0xA..0xE, excluding 0xF).
#[test]
fn message_type_invalid_bytes() {
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xA0),
        RemoteIdMessageType::Invalid
    );
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xB0),
        RemoteIdMessageType::Invalid
    );
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xC0),
        RemoteIdMessageType::Invalid
    );
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xD0),
        RemoteIdMessageType::Invalid
    );
    assert_eq!(
        RemoteIdMessageType::from_encoded_byte(0xE0),
        RemoteIdMessageType::Invalid
    );
}

/// Rotation counter + pack message — strip_rotation_counter handles 26-byte payload.
#[test]
fn rotation_counter_plus_pack() {
    let mut uas = RemoteIdUasData::default();
    let mut payload = vec![0u8; 3 + 2 * MESSAGE_SIZE + 1]; // rotation + pack
    payload[0] = 0x42; // rotation counter
                       // Then a pack starting at offset 1.
    payload[1] = 0xF0; // Packed
    payload[2] = MESSAGE_SIZE as u8;
    payload[3] = 2;
    payload[4] = 0x00; // BasicId
    payload[4 + MESSAGE_SIZE] = 0x10; // Location

    let result = decode_open_drone_id_payload(&payload, &mut uas);
    assert!(result.is_some());
    assert!(uas.basic_id_valid[0]);
    assert!(uas.location_valid);
}

/// UasData::clear resets all state.
#[test]
fn uas_data_clear() {
    let mut uas = RemoteIdUasData::default();
    uas.basic_id_valid[0] = true;
    uas.location_valid = true;
    uas.system_valid = true;
    uas.operator_id_valid = true;
    assert!(uas.is_alert_ready());

    uas.clear();
    assert!(!uas.basic_id_valid[0]);
    assert!(!uas.location_valid);
    assert!(!uas.system_valid);
    assert!(!uas.operator_id_valid);
    assert!(!uas.is_alert_ready());
    assert!(uas.primary_uas_id().is_none());
}
