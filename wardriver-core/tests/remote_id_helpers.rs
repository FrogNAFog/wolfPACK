//! Shared helpers for Remote ID host tests.

use wardriver_core::remote_id::types::{RemoteIdMessageType, MESSAGE_SIZE};

pub fn encode_lat_lon(deg: f64) -> i32 {
    (deg * 10_000_000.0).round() as i32
}

pub fn encode_altitude_m(m: f32) -> u16 {
    ((m + 1000.0) / 0.5).round() as u16
}

pub fn sample_basic_id_msg(uas_id: &[u8]) -> [u8; MESSAGE_SIZE] {
    let mut msg = [0u8; MESSAGE_SIZE];
    msg[0] = (RemoteIdMessageType::BasicId as u8) << 4;
    msg[1] = (1 << 4) | 2; // serial id type, multirotor
    let n = uas_id.len().min(20);
    msg[2..2 + n].copy_from_slice(&uas_id[..n]);
    msg
}

pub fn sample_location_msg(lat: f64, lon: f64, alt_m: f32) -> [u8; MESSAGE_SIZE] {
    let mut msg = [0u8; MESSAGE_SIZE];
    msg[0] = (RemoteIdMessageType::Location as u8) << 4;
    msg[1] = 2 << 4; // airborne
    msg[2] = 90; // direction
    msg[3] = 20; // speed
    msg[4] = 0; // vertical speed
    msg[5..9].copy_from_slice(&encode_lat_lon(lat).to_le_bytes());
    msg[9..13].copy_from_slice(&encode_lat_lon(lon).to_le_bytes());
    msg[15..17].copy_from_slice(&encode_altitude_m(alt_m).to_le_bytes());
    msg
}

pub fn sample_message_pack(msgs: &[[u8; MESSAGE_SIZE]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + msgs.len() * MESSAGE_SIZE);
    out.push((RemoteIdMessageType::Packed as u8) << 4);
    out.push(MESSAGE_SIZE as u8);
    out.push(msgs.len() as u8);
    for m in msgs {
        out.extend_from_slice(m);
    }
    out
}

pub fn beacon_with_vendor_ie(src: [u8; 6], odid_payload: &[u8]) -> Vec<u8> {
    let mut mpdu = Vec::new();
    mpdu.extend_from_slice(&[0x80, 0x00]); // beacon
    mpdu.extend_from_slice(&[0x00, 0x00]); // duration
    mpdu.extend_from_slice(&[0xFF; 6]); // DA broadcast
    mpdu.extend_from_slice(&src); // SA
    mpdu.extend_from_slice(&src); // BSSID
    mpdu.extend_from_slice(&[0x00, 0x00]); // seq
    mpdu.extend_from_slice(&[0u8; 8]); // timestamp
    mpdu.extend_from_slice(&[0x64, 0x00]); // beacon interval
    mpdu.extend_from_slice(&[0x11, 0x04]); // capability
                                           // SSID IE
    mpdu.push(0); // id
    mpdu.push(0); // len
                  // Vendor IE: FA:0B:BC + app 0x0D + ODID
    let vendor_len = 4 + odid_payload.len();
    mpdu.push(221);
    mpdu.push(vendor_len as u8);
    mpdu.extend_from_slice(&[0xFA, 0x0B, 0xBC, 0x0D]);
    mpdu.extend_from_slice(odid_payload);
    mpdu
}

pub fn ble_service_data_adv(odid_payload: &[u8]) -> Vec<u8> {
    let inner_len = 2 + 1 + odid_payload.len();
    let mut adv = Vec::new();
    adv.push((inner_len + 1) as u8);
    adv.push(0x16);
    adv.extend_from_slice(&[0xFA, 0xFF, 0x0D]);
    adv.extend_from_slice(odid_payload);
    adv
}
