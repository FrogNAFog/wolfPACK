//! Host tests for [`wardriver_core::peer_join_queue`].

use wardriver_core::peer_join_queue::{
    AdmitDecision, PeerJoinQueue, WaitReason, QUEUE_LIVENESS_MS,
};

#[test]
fn empty_queue_admits() {
    let mut q = PeerJoinQueue::new();
    assert_eq!(q.admit_or_enqueue(1, [1; 6], 1000), AdmitDecision::Admit);
}

#[test]
fn fifo_head_blocks_second_sniffer() {
    let mut q = PeerJoinQueue::new();
    let pos = q.enqueue_cap_full(10, [0xAA; 6], 5_000);
    assert_eq!(pos, 1);
    let d = q.admit_or_enqueue(20, [0xBB; 6], 5_001);
    assert!(matches!(
        d,
        AdmitDecision::Wait {
            reason: WaitReason::NotYourTurn,
            position: 2,
        }
    ));
}

#[test]
fn second_joiner_waits_after_head_enqueued() {
    let mut q = PeerJoinQueue::new();
    assert_eq!(q.admit_or_enqueue(1, [1; 6], 1000), AdmitDecision::Admit);
    q.enqueue_cap_full(1, [1; 6], 1000);
    let d = q.admit_or_enqueue(2, [2; 6], 1001);
    assert!(matches!(
        d,
        AdmitDecision::Wait {
            reason: WaitReason::NotYourTurn,
            position: 2,
        }
    ));
}

#[test]
fn head_refresh_admits() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1000);
    q.enqueue_cap_full(2, [2; 6], 1001);
    assert!(matches!(
        q.admit_or_enqueue(1, [1; 6], 2000),
        AdmitDecision::Admit
    ));
}

#[test]
fn expired_head_skipped() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1000);
    let expired = 1000 + QUEUE_LIVENESS_MS + 1;
    assert_eq!(q.prune_expired(expired), 1);
    assert_eq!(q.admit_or_enqueue(2, [2; 6], expired), AdmitDecision::Admit);
}

#[test]
fn live_waiter_count_excludes_expired() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1_000);
    assert_eq!(q.live_waiter_count(2_000), 1);
    assert_eq!(q.live_waiter_count(1_000 + QUEUE_LIVENESS_MS + 1), 0);
}

#[test]
fn cap_full_enqueue_keeps_fifo_order() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 100);
    q.enqueue_cap_full(2, [2; 6], 101);
    assert_eq!(q.live_waiter_count(200), 2);
    q.on_admitted(1, &[1; 6]);
    assert!(matches!(
        q.admit_or_enqueue(2, [2; 6], 201),
        AdmitDecision::Admit
    ));
}

#[test]
fn cap_full_enqueue_after_admitted_head_refresh() {
    let mut q = PeerJoinQueue::new();
    assert_eq!(q.admit_or_enqueue(1, [1; 6], 1000), AdmitDecision::Admit);
    q.on_admitted(1, &[1; 6]);
    let pos = q.enqueue_cap_full(1, [1; 6], 1001);
    assert_eq!(pos, 1);
    assert!(matches!(
        q.admit_or_enqueue(1, [1; 6], 1002),
        AdmitDecision::Admit
    ));
}

#[test]
fn on_admitted_removes_entry() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1000);
    q.on_admitted(1, &[1; 6]);
    assert_eq!(q.live_waiter_count(1000), 0);
}

#[test]
fn remove_sniffer_drops_fifo_entry() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1000);
    q.enqueue_cap_full(2, [2; 6], 1001);
    assert!(q.remove_sniffer(1));
    assert_eq!(q.live_waiter_count(2000), 1);
    assert!(!q.remove_sniffer(1));
    assert!(q.remove_sniffer(2));
    assert_eq!(q.live_waiter_count(2000), 0);
}
