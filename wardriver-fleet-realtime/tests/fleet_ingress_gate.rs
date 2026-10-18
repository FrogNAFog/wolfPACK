//! Outer ingress roster gate (control parity).

use wardriver_fleet_realtime::{bypass_roster_gate, ingress_roster_should_drop};
use wardriver_proto::FrameType;

#[test]
fn bypass_only_hello_and_peer_join() {
    assert!(bypass_roster_gate(FrameType::SnifferHello as u8));
    assert!(bypass_roster_gate(FrameType::PeerJoin as u8));
    assert!(!bypass_roster_gate(FrameType::SnifferAck as u8));
    assert!(!bypass_roster_gate(FrameType::ScanFrag as u8));
    assert!(!bypass_roster_gate(FrameType::TimeSyncPong as u8));
}

#[test]
fn hello_and_peer_join_never_dropped_without_roster() {
    assert!(!ingress_roster_should_drop(
        FrameType::SnifferHello as u8,
        false
    ));
    assert!(!ingress_roster_should_drop(
        FrameType::PeerJoin as u8,
        false
    ));
}

#[test]
fn ack_scanfrag_timesync_require_roster() {
    for ft in [
        FrameType::SnifferAck,
        FrameType::ScanFrag,
        FrameType::TimeSyncPong,
    ] {
        assert!(ingress_roster_should_drop(ft as u8, false));
        assert!(!ingress_roster_should_drop(ft as u8, true));
    }
}
