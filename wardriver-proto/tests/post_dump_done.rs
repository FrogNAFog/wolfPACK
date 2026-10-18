//! PROTO v6 PostDumpDone round-trip (wolfPACK).

use wardriver_proto::{
    pack_post_dump_done, unpack_post_dump_done, FrameType, PostDumpDoneV1, ESP_NOW_MAX_FRAME_LEN,
    POST_DUMP_DONE_FRAME_LEN, PROTO_VERSION,
};

#[test]
fn post_dump_done_v1_roundtrip() {
    let done = PostDumpDoneV1 {
        sniffer_id: 7,
        session_seq: 42,
        control_tx_ms: 1_234_567,
    };
    let wire = pack_post_dump_done(done);
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire[1], FrameType::PostDumpDone as u8);
    assert_eq!(wire.len(), POST_DUMP_DONE_FRAME_LEN);
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let back = unpack_post_dump_done(&wire).expect("unpack post dump done");
    assert_eq!(back, done);
}

// --- Tests migrated from wire_edge.rs ---

/// PostDumpDone roundtrip (wire-level variant).
#[test]
fn post_dump_done_wire_roundtrip() {
    let done = PostDumpDoneV1 {
        sniffer_id: 7,
        session_seq: 42,
        control_tx_ms: 100_000,
    };
    let frame = pack_post_dump_done(done);
    let unpacked = unpack_post_dump_done(&frame).expect("roundtrip");
    assert_eq!(unpacked.sniffer_id, 7);
    assert_eq!(unpacked.session_seq, 42);
    assert_eq!(unpacked.control_tx_ms, 100_000);
}

/// PostDumpDone unpack with short frame.
#[test]
fn unpack_post_dump_done_short_frame() {
    let short = vec![0u8; POST_DUMP_DONE_FRAME_LEN - 1];
    assert!(unpack_post_dump_done(&short).is_err());
}
