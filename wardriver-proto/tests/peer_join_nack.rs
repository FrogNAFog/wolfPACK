//! PROTO v6 PeerJoinNack round-trip (wolfPACK).

use wardriver_proto::{
    pack_peer_join_nack, unpack_peer_join_nack, FrameType, PeerJoinNackReason, PeerJoinNackV1,
    PEER_JOIN_NACK_FRAME_LEN, PROTO_VERSION,
};

#[test]
fn peer_join_nack_v1_roundtrip() {
    for (reason, pos) in [
        (PeerJoinNackReason::CapFull, 1u8),
        (PeerJoinNackReason::NotYourTurn, 3),
    ] {
        let nack = PeerJoinNackV1 {
            sniffer_id: 42,
            reason,
            retry_after_ms: 1_000,
            queue_position: pos,
        };
        let wire = pack_peer_join_nack(nack);
        assert_eq!(wire.len(), PEER_JOIN_NACK_FRAME_LEN);
        assert_eq!(wire[0], PROTO_VERSION);
        assert_eq!(wire[1], FrameType::PeerJoinNack as u8);
        let back = unpack_peer_join_nack(&wire).expect("unpack peer join nack");
        assert_eq!(back.sniffer_id, nack.sniffer_id);
        assert_eq!(back.reason as u8, nack.reason as u8);
        assert_eq!(back.retry_after_ms, nack.retry_after_ms);
        assert_eq!(back.queue_position, nack.queue_position);
    }
}

#[test]
fn peer_join_nack_rejects_unknown_reason() {
    let mut wire = pack_peer_join_nack(PeerJoinNackV1 {
        sniffer_id: 1,
        reason: PeerJoinNackReason::CapFull,
        retry_after_ms: 500,
        queue_position: 1,
    });
    wire[14] = 99;
    assert!(unpack_peer_join_nack(&wire).is_err());
}

// --- Tests migrated from wire_edge.rs ---

/// PeerJoinNack roundtrip with CapFull reason (explicit values).
#[test]
fn peer_join_nack_cap_full_roundtrip() {
    let nack = PeerJoinNackV1 {
        sniffer_id: 5,
        reason: PeerJoinNackReason::CapFull,
        retry_after_ms: 1_000,
        queue_position: 1,
    };
    let frame = pack_peer_join_nack(nack);
    let unpacked = unpack_peer_join_nack(&frame).expect("roundtrip");
    assert_eq!(unpacked.sniffer_id, 5);
    assert_eq!(unpacked.reason, PeerJoinNackReason::CapFull);
    assert_eq!(unpacked.retry_after_ms, 1_000);
    assert_eq!(unpacked.queue_position, 1);
}

/// PeerJoinNack with NotYourTurn reason roundtrip.
#[test]
fn peer_join_nack_not_your_turn_roundtrip() {
    let nack = PeerJoinNackV1 {
        sniffer_id: 3,
        reason: PeerJoinNackReason::NotYourTurn,
        retry_after_ms: 2_000,
        queue_position: 2,
    };
    let frame = pack_peer_join_nack(nack);
    let unpacked = unpack_peer_join_nack(&frame).expect("roundtrip");
    assert_eq!(unpacked.reason, PeerJoinNackReason::NotYourTurn);
    assert_eq!(unpacked.queue_position, 2);
}

/// PeerJoinNack unpack with short frame.
#[test]
fn peer_join_nack_unpack_short_frame() {
    let short = vec![0u8; PEER_JOIN_NACK_FRAME_LEN - 1];
    assert!(unpack_peer_join_nack(&short).is_err());
}
