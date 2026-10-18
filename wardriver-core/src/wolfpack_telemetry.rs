//! Pure wolfPACK dump-visit duration math (host-testable; mirrors control `peer_pool`).

/// Completed dump-visit phase durations (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WolfpackDumpDurations {
    /// First `PeerJoin` → `PeerJoinAck` (FIFO wait).
    pub queue_ms: u64,
    /// First bulk frag → `TRANSPORT_END` (encrypted wardrive transfer).
    pub bulk_xfer_ms: u64,
    /// `TRANSPORT_END` → `PostDumpDone` (post-flush + done delay).
    pub post_ms: u64,
    /// `PeerJoinAck` → `PostDumpDone` (active visit on control).
    pub visit_ms: u64,
}

/// Derive per-phase durations from control post-dump session timestamps.
///
/// `join_attempt_ms` is the wall time of the first `PeerJoin` for this visit;
/// `visit_start_ms` is set on successful `PeerJoinAck`. All other anchors match
/// [`wardriver_control::control::peer_pool::PostDumpSession`] field semantics.
#[must_use]
pub fn wolfpack_dump_durations(
    join_attempt_ms: u64,
    visit_start_ms: u64,
    bulk_start_ms: u64,
    transport_end_ms: u64,
    done_ms: u64,
) -> WolfpackDumpDurations {
    let queue_ms = if join_attempt_ms != 0 && visit_start_ms >= join_attempt_ms {
        visit_start_ms.saturating_sub(join_attempt_ms)
    } else {
        0
    };
    let bulk_xfer_ms = if bulk_start_ms != 0 && transport_end_ms >= bulk_start_ms {
        transport_end_ms.saturating_sub(bulk_start_ms)
    } else {
        0
    };
    let post_ms = if transport_end_ms != 0 {
        done_ms.saturating_sub(transport_end_ms)
    } else {
        0
    };
    let visit_ms = if visit_start_ms != 0 {
        done_ms.saturating_sub(visit_start_ms)
    } else {
        0
    };
    WolfpackDumpDurations {
        queue_ms,
        bulk_xfer_ms,
        post_ms,
        visit_ms,
    }
}
