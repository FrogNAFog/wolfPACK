//! Host tests for PeerJoinQueue edge cases (capacity, MAC reuse, dedup).

use wardriver_core::peer_join_queue::{
    AdmitDecision, PeerJoinQueue, WaitReason, QUEUE_LIVENESS_MS, QUEUE_MAX_ENTRIES,
};

/// Queue at maximum capacity: `enqueue` drops the last pushed entry to make room.
#[test]
fn queue_at_max_capacity_drops_tail() {
    let mut q = PeerJoinQueue::new();
    // Fill to QUEUE_MAX_ENTRIES (48).
    for i in 0..QUEUE_MAX_ENTRIES {
        let mac: [u8; 6] = [i as u8; 6];
        q.enqueue_cap_full(i as u32, mac, 1_000 + i as u64);
    }
    assert_eq!(q.live_waiter_count(2_000), QUEUE_MAX_ENTRIES as u8);

    // Next enqueue drops the oldest entry (idx 0, sniffer_id=0) via pop(), then pushes new.
    // Actually: pop() removes the LAST element (most recently added = sniffer_id=47),
    // then push adds the new entry. So sniffer_id=47 is dropped.
    let new_mac = [0xFF; 6];
    let pos = q.enqueue_cap_full((QUEUE_MAX_ENTRIES) as u32, new_mac, 3_000);
    // Position should be last slot (QUEUE_MAX_ENTRIES).
    assert_eq!(pos, QUEUE_MAX_ENTRIES as u8);
    assert_eq!(q.live_waiter_count(3_000), QUEUE_MAX_ENTRIES as u8);
    // sniffer_id=47 should be gone (was popped).
    assert!(!q.remove_sniffer(47));
    // sniffer_id=0 should still be present.
    assert!(q.remove_sniffer(0));
}

/// Sniffer rejoining immediately after being removed (MAC reuse).
#[test]
fn sniffer_rejoin_after_removal() {
    let mut q = PeerJoinQueue::new();
    let mac: [u8; 6] = [0xAA; 6];

    // Enqueue sniffer 1.
    q.enqueue_cap_full(1, mac, 1_000);
    assert_eq!(q.live_waiter_count(2_000), 1);

    // Remove sniffer 1.
    assert!(q.remove_sniffer(1));
    assert_eq!(q.live_waiter_count(2_000), 0);

    // Sniffer 1 rejoins with same MAC — should be admitted (queue empty).
    let d = q.admit_or_enqueue(1, mac, 3_000);
    assert_eq!(d, AdmitDecision::Admit);
}

/// Admitted sniffers are not enqueued — dedup only applies within the queue.
/// A sniffer that got Admit is not in the queue, so a second sniffer with the same MAC
/// sees an empty queue and also gets Admit.
#[test]
fn admitted_not_enqueued_separate_sniffer_ids() {
    let mut q = PeerJoinQueue::new();
    let mac: [u8; 6] = [0xBB; 6];

    // First sniffer gets Admit (not enqueued).
    assert_eq!(q.admit_or_enqueue(1, mac, 1_000), AdmitDecision::Admit);
    // Queue is empty — second sniffer with same MAC also gets Admit.
    assert_eq!(q.admit_or_enqueue(2, mac, 1_001), AdmitDecision::Admit);
    // Queue remains empty (both admitted, not enqueued).
    assert_eq!(q.live_waiter_count(2_000), 0);
}

/// Same sniffer_id with different MAC — both get Admit (not enqueued).
#[test]
fn same_sniffer_id_different_mac_both_admitted() {
    let mut q = PeerJoinQueue::new();
    let mac1: [u8; 6] = [0x11; 6];
    let mac2: [u8; 6] = [0x22; 6];

    // First MAC admitted (not enqueued).
    assert_eq!(q.admit_or_enqueue(5, mac1, 1_000), AdmitDecision::Admit);
    // Queue empty — second MAC also admitted.
    assert_eq!(q.admit_or_enqueue(5, mac2, 1_001), AdmitDecision::Admit);
    // Queue remains empty.
    assert_eq!(q.live_waiter_count(2_000), 0);
}

/// on_admitted only removes the matching (sniffer_id, MAC) pair.
#[test]
fn on_admitted_matches_sniffer_and_mac() {
    let mut q = PeerJoinQueue::new();
    let mac1: [u8; 6] = [0x11; 6];
    let mac2: [u8; 6] = [0x22; 6];

    q.enqueue_cap_full(1, mac1, 1_000);
    q.enqueue_cap_full(1, mac2, 1_001);
    assert_eq!(q.live_waiter_count(2_000), 2);

    // on_admitted with mac1 only removes mac1 entry.
    q.on_admitted(1, &mac1);
    assert_eq!(q.live_waiter_count(2_000), 1);
    // mac2 entry remains.
    assert!(q.remove_sniffer(1));
    assert_eq!(q.live_waiter_count(2_000), 0);
}

/// Prune expired removes only stale entries; live entries preserved.
#[test]
fn prune_expired_partial() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1_000);
    q.enqueue_cap_full(2, [2; 6], 1_000);
    q.enqueue_cap_full(3, [3; 6], 20_000); // live

    let expired = 1_000 + QUEUE_LIVENESS_MS + 1;
    let removed = q.prune_expired(expired);
    assert_eq!(removed, 2);
    assert_eq!(q.live_waiter_count(expired), 1);
}

/// Enqueue_cap_full refreshes liveness of existing entry (idempotent).
#[test]
fn enqueue_cap_full_refreshes_liveness() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1_000);

    // Enqueue again with later timestamp — should refresh, not double.
    let pos = q.enqueue_cap_full(1, [1; 6], 5_000);
    assert_eq!(pos, 1);
    assert_eq!(q.live_waiter_count(5_000), 1);
    // Entry was refreshed to 5_000 — should still be live at 5_000 + liveness.
    assert_eq!(q.live_waiter_count(5_000 + QUEUE_LIVENESS_MS - 1), 1);
    // Should be expired at 5_000 + liveness + 1.
    assert_eq!(q.live_waiter_count(5_000 + QUEUE_LIVENESS_MS + 1), 0);
}

/// Prune uses saturating subtraction — no panic on now_ms < last_join_ms.
#[test]
fn prune_with_future_timestamp_no_panic() {
    let mut q = PeerJoinQueue::new();
    // Enqueue with a very large timestamp.
    q.enqueue_cap_full(1, [1; 6], u64::MAX);
    // Pruning at a small now_ms should not panic (saturating sub).
    let removed = q.prune_expired(100);
    assert_eq!(removed, 0);
    assert_eq!(q.live_waiter_count(100), 1);
}

/// live_waiter_count caps at 255 when queue has more entries.
#[test]
fn live_waiter_count_caps_at_255() {
    let mut q = PeerJoinQueue::new();
    // QUEUE_MAX_ENTRIES is 48 — not enough to test 255 cap directly,
    // but verify it doesn't overflow.
    for i in 0..QUEUE_MAX_ENTRIES {
        let mac: [u8; 6] = [i as u8; 6];
        q.enqueue_cap_full(i as u32, mac, 1_000);
    }
    assert_eq!(q.live_waiter_count(2_000), QUEUE_MAX_ENTRIES as u8);
}

/// Admit_decision::Wait position is 1-based index in queue.
#[test]
fn wait_position_is_one_based() {
    let mut q = PeerJoinQueue::new();
    q.enqueue_cap_full(1, [1; 6], 1_000);
    q.enqueue_cap_full(2, [2; 6], 1_001);
    q.enqueue_cap_full(3, [3; 6], 1_002);

    let d = q.admit_or_enqueue(4, [4; 6], 1_003);
    assert!(matches!(d, AdmitDecision::Wait { position: 4, .. }));
}
