//! Shared prologue for ESP-NOW alert-frame handlers.
//!
//! The detection (WiFi / BLE), remote-ID, and probe alert handlers each begin with the
//! same sequence: RX-counter bump, proto unpack, sniffer liveness, roster adoption sync,
//! sniffer-boot-id freshness gate, method-disable mask gate, detection-ignore BSSID filter,
//! and alert-log dedup. [`alert_prologue`] factors that prefix into one call so each
//! handler keeps only its unique post-gate action.
//!
//! `probe` (probe-request burst follow-ups) intentionally does **not** use this helper: it is
//! a bulk-telemetry handler with no boot-id / method-mask / dedup gates, so forcing it through
//! the alert prologue would add gates that were never there.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};

use super::csv_ingest::empty_bssid_set;
use super::ingress::sync_adoption_from_roster;
use super::ingress_ctx::ControlIngressCtx;
use wardriver_stats as stats;

/// Run the common alert-frame prologue and return the unpacked payload when every gate passes.
///
/// Order is fixed and mirrors the original inline handlers exactly:
/// RX counter → unpack → liveness → roster sync → boot-id gate → method-disable gate →
/// detection-ignore filter → alert-log dedup. A `None` return means the frame was dropped
/// (or failed to unpack) and the caller must do nothing.
///
/// `is_disabled` receives the method mask plus the unpacked item (remote-ID selects the
/// WiFi/BLE disable table by `signal_kind`); the only closure borrowing `ctx` is
/// `dedup_hit`, which runs last.
pub(super) fn alert_prologue<T>(
    ctx: &mut ControlIngressCtx<'_>,
    peer: [u8; 6],
    data: &[u8],
    rx_counter: &AtomicU32,
    mask: u32,
    is_disabled: impl Fn(u32, &T) -> bool,
    check_detection_ignore: bool,
    unpack: impl FnOnce(&[u8]) -> anyhow::Result<T>,
    sniffer_id_of: impl Fn(&T) -> u32,
    boot_id_of: impl Fn(&T) -> u32,
    src_of: impl Fn(&T) -> &[u8; 6],
    after_ignore: impl Fn(&T),
    dedup_hit: impl Fn(&mut ControlIngressCtx<'_>, &T) -> bool,
) -> Option<T> {
    rx_counter.fetch_add(1, Ordering::Relaxed);
    let Ok(item) = unpack(data) else {
        return None;
    };
    let sniffer_id = sniffer_id_of(&item);
    stats::sniffer_seen_liveness(sniffer_id, &peer);
    sync_adoption_from_roster(sniffer_id, &peer);
    if let Some(boot) = ctx.last_sniffer_boot_id.get(&sniffer_id).copied() {
        if boot != boot_id_of(&item) {
            return None;
        }
    }
    if is_disabled(mask, &item) {
        return None;
    }
    if check_detection_ignore {
        let detection_ignore = ctx.detection_ignore.read().ok();
        let detection_ignore_ref: &HashSet<[u8; 6]> = match detection_ignore.as_deref() {
            Some(s) => s,
            None => empty_bssid_set(),
        };
        if detection_ignore_ref.contains(src_of(&item)) {
            return None;
        }
    }
    after_ignore(&item);
    if dedup_hit(ctx, &item) {
        return None;
    }
    Some(item)
}
