//! Host tests for [`wardriver_core::wolfpack_telemetry`].

use wardriver_core::wolfpack_telemetry::{wolfpack_dump_durations, WolfpackDumpDurations};

#[test]
fn queue_ms_is_join_to_ack() {
    let d = wolfpack_dump_durations(1000, 1250, 0, 0, 2000);
    assert_eq!(
        d,
        WolfpackDumpDurations {
            queue_ms: 250,
            bulk_xfer_ms: 0,
            post_ms: 0,
            visit_ms: 750,
        }
    );
}

#[test]
fn bulk_xfer_ms_is_frag_to_transport_end() {
    let d = wolfpack_dump_durations(1000, 1200, 1300, 1800, 2000);
    assert_eq!(
        d,
        WolfpackDumpDurations {
            queue_ms: 200,
            bulk_xfer_ms: 500,
            post_ms: 200,
            visit_ms: 800,
        }
    );
}

#[test]
fn zero_frags_visit_has_no_bulk_xfer() {
    let d = wolfpack_dump_durations(500, 700, 0, 900, 1000);
    assert_eq!(
        d,
        WolfpackDumpDurations {
            queue_ms: 200,
            bulk_xfer_ms: 0,
            post_ms: 100,
            visit_ms: 300,
        }
    );
}

#[test]
fn missing_join_attempt_yields_zero_queue() {
    let d = wolfpack_dump_durations(0, 1000, 1100, 1500, 1600);
    assert_eq!(d.queue_ms, 0);
    assert_eq!(d.bulk_xfer_ms, 400);
}
