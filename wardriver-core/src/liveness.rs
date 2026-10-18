//! Shared uplink liveness budget (control peer eviction + sniffer link-dead).

/// Control roster `last_seen_ms` staleness and sniffer link-dead budget (one knob).
pub const SNIFFER_UPLINK_LIVENESS_MS: u64 = 20_000;

/// Reboot when control RX snapshot is unchanged for twice the liveness budget.
pub const SNIFFER_LINK_REBOOT_MS: u64 = SNIFFER_UPLINK_LIVENESS_MS.saturating_mul(2);

/// True when an encrypted peer row should be evicted for uplink staleness.
#[must_use]
pub fn stale_uplink_peer_should_evict(last_seen_ms: u64, now_ms: u64) -> bool {
    if last_seen_ms == 0 {
        return true;
    }
    now_ms.saturating_sub(last_seen_ms) > SNIFFER_UPLINK_LIVENESS_MS
}

/// Milliseconds since link-dead timer was armed (`None` when RX is advancing).
#[must_use]
pub fn link_dead_elapsed_ms(link_dead_armed_at_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    link_dead_armed_at_ms.map(|armed| now_ms.saturating_sub(armed))
}

/// True when sniffer should enter link-dead recovery (single path).
#[must_use]
pub fn link_dead_due(link_dead_armed_at_ms: Option<u64>, now_ms: u64) -> bool {
    link_dead_elapsed_ms(link_dead_armed_at_ms, now_ms)
        .is_some_and(|elapsed| elapsed >= SNIFFER_UPLINK_LIVENESS_MS)
}

/// True when sniffer should reboot after prolonged RX stagnation.
#[must_use]
pub fn link_reboot_due(link_dead_armed_at_ms: Option<u64>, now_ms: u64) -> bool {
    link_dead_elapsed_ms(link_dead_armed_at_ms, now_ms)
        .is_some_and(|elapsed| elapsed >= SNIFFER_LINK_REBOOT_MS)
}

// --- Mobile / control web sniffer roster display (matches `SNIFFER_QUIET_STALE_MS`) ---

/// Gateway-relative stale threshold (control web + Android `SnifferFormatting`).
pub const SNIFFER_QUIET_STALE_MS: u64 = SNIFFER_UPLINK_LIVENESS_MS;

/// Control-style stale: `gateway_now_ms - last_seen_ms >= 20s`.
#[must_use]
pub fn sniffer_is_quiet_stale(
    gateway_now_ms: u64,
    last_seen_ms: u64,
    gateway_connected: bool,
) -> bool {
    if !gateway_connected || gateway_now_ms == 0 || last_seen_ms == 0 {
        return false;
    }
    gateway_now_ms.saturating_sub(last_seen_ms) >= SNIFFER_QUIET_STALE_MS
}

/// Unadopted Nearby row visibility: connected and heard within the liveness window.
#[must_use]
pub fn pending_nearby_visible(
    gateway_now_ms: u64,
    last_seen_ms: u64,
    gateway_connected: bool,
) -> bool {
    if !gateway_connected || gateway_now_ms == 0 || last_seen_ms == 0 {
        return false;
    }
    gateway_now_ms.saturating_sub(last_seen_ms) < SNIFFER_QUIET_STALE_MS
}

/// True when a Nearby row is shown but last heard exceeds the liveness window.
#[must_use]
pub fn pending_nearby_stale(
    gateway_now_ms: u64,
    last_seen_ms: u64,
    gateway_connected: bool,
) -> bool {
    !pending_nearby_visible(gateway_now_ms, last_seen_ms, gateway_connected)
        && gateway_connected
        && gateway_now_ms > 0
        && last_seen_ms > 0
}

/// Control-style relative last-seen (second precision under 1 minute).
#[must_use]
pub fn format_sniffer_last_seen_ago(gateway_now_ms: u64, last_seen_ms: Option<u64>) -> String {
    let Some(last_seen_ms) = last_seen_ms else {
        return "-".to_string();
    };
    if last_seen_ms == 0 {
        return "-".to_string();
    }
    let d = gateway_now_ms.saturating_sub(last_seen_ms);
    if gateway_now_ms < last_seen_ms || d > 86_400_000 {
        return "-".to_string();
    }
    if d < 1500 {
        return "just now".to_string();
    }
    let sec = d / 1000;
    if sec < 60 {
        return format!("{sec}s ago");
    }
    format!("{}m ago", sec / 60)
}

/// wolfPACK `SnifferState` wire label.
#[must_use]
pub fn state_label(state: Option<u8>) -> &'static str {
    match state {
        Some(1) => "SCANNING",
        Some(0) => "IDLE",
        None => "-",
        Some(_) => "ERROR",
    }
}

/// Short display ID (last 4 hex digits of u32).
#[must_use]
pub fn short_sniffer_id(sniffer_id: u32) -> String {
    let hex = format!("{:08X}", sniffer_id);
    format!("...{}", &hex[hex.len() - 4..])
}
