//! PackLink SoftAP TOFU handshake and SoftApConfig host tests.

use wardriver_packlink::{
    pack_phone_hello, pack_softap_config, unpack_frame, unpack_phone_hello, unpack_softap_config,
    MsgType, PhoneHelloV1, SoftApConfigV1,
};

#[test]
fn phone_hello_roundtrip() {
    let hello = PhoneHelloV1 {
        phone_id: "550e8400-e29b-41d4-a716-446655440000".into(),
    };
    let wire = pack_phone_hello(&hello, 1);
    let frame = unpack_frame(&wire).expect("phone_hello frame");
    assert_eq!(frame.msg_type, MsgType::PhoneHello);
    let back = unpack_phone_hello(frame.payload).expect("phone_hello payload");
    assert_eq!(back, hello);
}

#[test]
fn softap_config_roundtrip() {
    let cfg = SoftApConfigV1 {
        ssid: "wolfPACK".into(),
        password: "deflock123".into(),
    };
    let wire = pack_softap_config(&cfg, 4);
    let frame = unpack_frame(&wire).expect("softap_config frame");
    assert_eq!(frame.msg_type, MsgType::SoftApConfig);
    let back = unpack_softap_config(frame.payload).expect("softap_config payload");
    assert_eq!(back, cfg);
}
