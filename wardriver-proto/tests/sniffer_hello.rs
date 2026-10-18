//! SnifferHello pack/unpack round-trip (all PROTO versions).

use wardriver_proto::{
    pack_sniffer_hello, unpack_sniffer_hello, FrameType, SnifferHelloV1, SnifferRole,
    PROTO_VERSION, SNIFFER_HELLO_FRAME_LEN,
};

#[test]
fn sniffer_hello_v1_roundtrip() {
    let hello = SnifferHelloV1 {
        sniffer_id: 7,
        sniffer_boot_id: 0xDEAD_BEEF,
        role: SnifferRole::Wifi,
        ble_capable: 1,
        five_ghz_capable: 0,
        dfs_capable: 1,
        proto_min: PROTO_VERSION,
        proto_max: PROTO_VERSION,
    };
    let wire = pack_sniffer_hello(hello);
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire[1], FrameType::SnifferHello as u8);
    assert_eq!(wire.len(), SNIFFER_HELLO_FRAME_LEN);
    let back = unpack_sniffer_hello(&wire).expect("unpack sniffer hello");
    assert_eq!(back, hello);
}

/// Regression: 24-byte wire must pass the unpack length gate (was wrongly 25).
#[test]
fn sniffer_hello_exact_24_byte_wire_unpacks() {
    assert_eq!(SNIFFER_HELLO_FRAME_LEN, 24);
    let mut wire: [u8; 24] = [
        0x04, 0x0e, 0x57, 0x44, 0x52, 0x56, 0x52, 0x44, 0x57, 0x32, 0x01, 0x00, 0x00, 0x00, 0x00,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x04,
    ];
    wire[0] = PROTO_VERSION;
    wire[22] = PROTO_VERSION;
    wire[23] = PROTO_VERSION;
    unpack_sniffer_hello(&wire).expect("24-byte hello must unpack");
}
