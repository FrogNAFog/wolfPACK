//! Sniffer roster helpers for channel plan.

use wardriver_core::liveness::SNIFFER_UPLINK_LIVENESS_MS;
use wardriver_stats as stats;

/// Time window (since last uplink liveness) during which a sniffer is considered active.
pub(crate) const ACTIVE_WINDOW_MS: u64 = SNIFFER_UPLINK_LIVENESS_MS;

/// WiFi channel planning: only **WiFi-role** sniffers (excludes BLE specialists).
pub(crate) fn fill_active_wifi_sniffers(out: &mut Vec<(u32, [u8; 6])>, now_ms: u64) {
    out.clear();
    stats::for_each_sniffer_slot(|s| {
        if s.role != 0 {
            return;
        }
        if now_ms.saturating_sub(s.last_seen_ms) > ACTIVE_WINDOW_MS {
            return;
        }
        out.push((s.sniffer_id, s.mac));
    });
    out.sort_by_key(|(id, _)| *id);
}
