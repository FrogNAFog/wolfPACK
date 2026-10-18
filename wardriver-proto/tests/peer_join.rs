//! PROTO v6 PeerJoin / PeerJoinAck round-trips (wolfPACK).

use wardriver_proto::{
    pack_peer_join, pack_peer_join_ack, unpack_peer_join, unpack_peer_join_ack, FrameType,
    PeerJoinAckV1, PeerJoinReason, PeerJoinV1, ESP_NOW_MAX_FRAME_LEN, PEER_JOIN_ACK_FRAME_LEN,
    PEER_JOIN_FRAME_LEN, PROTO_VERSION,
};

#[test]
fn peer_join_v1_roundtrip_all_reasons() {
    for (reason, id) in [
        (PeerJoinReason::DumpWifi, 1u32),
        (PeerJoinReason::DumpBle, 2),
        (PeerJoinReason::DumpProbe, 3),
        (PeerJoinReason::DetectionAlert, 4),
        (PeerJoinReason::PeerRefresh, 5),
    ] {
        let join = PeerJoinV1 {
            sniffer_id: id,
            sniffer_boot_id: 0xAABB_0000 | id,
            join_reason: reason,
        };
        let wire = pack_peer_join(join);
        assert_eq!(wire[0], PROTO_VERSION);
        assert_eq!(wire[1], FrameType::PeerJoin as u8);
        assert_eq!(wire.len(), PEER_JOIN_FRAME_LEN);
        assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
        let back = unpack_peer_join(&wire).expect("unpack peer join");
        assert_eq!(back, join);
    }
}

#[test]
fn peer_join_ack_v1_roundtrip() {
    let ack = PeerJoinAckV1 {
        sniffer_id: 12,
        fleet_slot: 37,
        peer_epoch: 0x1234_5678,
    };
    let wire = pack_peer_join_ack(ack);
    assert_eq!(wire[0], PROTO_VERSION);
    assert_eq!(wire[1], FrameType::PeerJoinAck as u8);
    assert_eq!(wire.len(), PEER_JOIN_ACK_FRAME_LEN);
    assert!(wire.len() <= ESP_NOW_MAX_FRAME_LEN);
    let back = unpack_peer_join_ack(&wire).expect("unpack peer join ack");
    assert_eq!(back, ack);
}

#[test]
fn peer_join_rejects_unknown_reason() {
    let mut wire = pack_peer_join(PeerJoinV1 {
        sniffer_id: 1,
        sniffer_boot_id: 2,
        join_reason: PeerJoinReason::DumpWifi,
    });
    wire[18] = 99;
    assert!(unpack_peer_join(&wire).is_err());
}

// --- Test migrated from wire_edge.rs (renamed to avoid collision) ---

/// PeerJoin with unknown join_reason (value 100) rejected via frame construction.
#[test]
fn peer_join_rejects_unknown_reason_via_frame() {
    let len = PEER_JOIN_FRAME_LEN;
    let mut frame = vec![0u8; len];
    frame[0] = PROTO_VERSION;
    frame[1] = 19;
    frame[2..6].copy_from_slice(b"WDRV");
    frame[6..10].copy_from_slice(&0x3257_4452u32.to_le_bytes());
    frame[10..14].copy_from_slice(&1u32.to_le_bytes());
    frame[14..18].copy_from_slice(&1u32.to_le_bytes());
    frame[18] = 100;
    assert!(unpack_peer_join(&frame).is_err());
}
