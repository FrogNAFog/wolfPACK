//! Control wrapper: post-dump RTT TimeSync in `wardriver-fleet-realtime`.

pub use wardriver_fleet_realtime::time_sync_rtt::{
    handle_time_sync_pong, maybe_start_post_dump_time_sync, post_dump_time_sync_ready,
    register_wall_clock, rtt_sync_due, WallClockHooks, TIME_SYNC_FALLBACK_MS,
    TIME_SYNC_RTT_INTERVAL_MS,
};

/// Register control GPS/SNTP wall-clock hooks (call once at boot).
pub fn register_control_wall_clock() {
    register_wall_clock(WallClockHooks {
        trusted: || crate::time_sync::wall_clock_trusted(),
        trusted_since_ms: || crate::time_sync::wall_clock_trusted_since_ms(),
        unix_now_ms: || super::csv_ingest::unix_now_ms(),
    });
}
