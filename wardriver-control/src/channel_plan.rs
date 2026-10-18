//! Operator-controlled channel plan for the wolfPACK sniffer fleet.
//!
//! Planning types and [`effective_from_plan`] live in [`wardriver_core::channel_plan`].
//! This module holds the control-side RAM singleton and SD load/save wrappers.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use log::{info, warn};

pub use wardriver_core::channel_plan::*;

// ---- Control-side process singleton ---------------------------------------------------------

static PLAN: Mutex<Option<ChannelPlan>> = Mutex::new(None);
static REVISION: AtomicU32 = AtomicU32::new(0);
static SD_LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn with_plan<R>(f: impl FnOnce(&mut ChannelPlan) -> R) -> R {
    let mut guard = PLAN.lock().expect("channel_plan: PLAN poisoned");
    if guard.is_none() {
        *guard = Some(ChannelPlan::default());
    }
    f(guard.as_mut().expect("channel_plan: PLAN init"))
}

/// Snapshot the active channel plan (cheap clone — small `Vec`s).
pub fn current_plan() -> ChannelPlan {
    with_plan(|p| p.clone())
}

/// Current revision (monotonic; bumped on every successful save/replace).
pub fn revision() -> u32 {
    REVISION.load(Ordering::Acquire)
}

/// Replace the in-memory plan with `new`. Bumps and returns the new revision; updates
/// [`wardriver_stats::set_channel_plan_revision`].
pub fn set_plan(new: ChannelPlan) -> u32 {
    let mut guard = PLAN.lock().expect("channel_plan: PLAN poisoned");
    *guard = Some(new);
    let rev = REVISION.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    wardriver_stats::set_channel_plan_revision(rev);
    rev
}

/// Apply channel plan from worker-collected boot snapshot (no SD VFS on main).
pub fn apply_from_boot_snapshot(text: Option<&str>) {
    if SD_LOADED.swap(true, Ordering::AcqRel) {
        return;
    }
    match text {
        Some(t) => match ChannelPlan::decode_kv(t) {
            Ok(plan) => {
                info!(
                    "channel_plan: loaded SD file (2_4={} 5={} dfs={} dwell={}ms)",
                    plan.include_2_4_ghz, plan.include_5_ghz, plan.include_dfs, plan.dwell_ms
                );
                let mut guard = PLAN.lock().expect("channel_plan: PLAN poisoned");
                *guard = Some(plan);
            }
            Err(e) => {
                warn!("channel_plan: SD decode failed ({e:#}); using defaults");
                wardriver_stats::note_channel_plan_sd_load_fail();
            }
        },
        None => info!("channel_plan: no SD file yet; using defaults"),
    }
}

/// Persist `plan` to SD atomically. Returns `Ok(())` on success; increments the SD save fail
/// counter and returns the error otherwise. Caller should still update the in-memory plan
/// (the SD write is the durability guarantee, not the source of truth at runtime).
pub fn save_to_sd(plan: &ChannelPlan, sd_io: &crate::storage::SdIoHandle) -> std::io::Result<()> {
    let text = plan.encode_kv();
    match crate::storage::write_channel_plan_text(sd_io, &text) {
        Ok(()) => Ok(()),
        Err(e) => {
            wardriver_stats::note_channel_plan_sd_save_fail();
            Err(e)
        }
    }
}

/// Reset the in-memory channel plan to factory defaults (same as first-boot state),
/// clearing all per-sniffer band assignments and pins. Bumps the revision so the
/// control node re-sends schedules to the fleet. Persists the defaults to SD when mounted.
///
/// Returns `(revision, sd_saved)` where `sd_saved` is false when the SD write failed or
/// SD was not mounted.
pub fn reset_to_defaults(sd_io: &crate::storage::SdIoHandle) -> (u32, bool) {
    let new = ChannelPlan::default();
    let rev = set_plan(new);
    let sd_saved = save_to_sd(&current_plan(), sd_io).is_ok();
    if !sd_saved {
        warn!("channel_plan: reset SD save failed (defaults active in RAM only)");
    }
    (rev, sd_saved)
}

/// Remove `sniffer_id` from manual band lists and per-sniffer pins; bumps revision when changed.
pub fn remove_sniffer_from_plan_lists(sniffer_id: u32, sd_io: &crate::storage::SdIoHandle) {
    if sniffer_id == 0 {
        return;
    }
    let mut plan = current_plan();
    let before_len = plan.sniffers_2_4.len() + plan.sniffers_5.len() + plan.per_sniffer.len();
    plan.sniffers_2_4.retain(|id| *id != sniffer_id);
    plan.sniffers_5.retain(|id| *id != sniffer_id);
    plan.per_sniffer.retain(|p| p.sniffer_id != sniffer_id);
    let after_len = plan.sniffers_2_4.len() + plan.sniffers_5.len() + plan.per_sniffer.len();
    if before_len == after_len {
        return;
    }
    set_plan(plan);
    if let Err(e) = save_to_sd(&current_plan(), sd_io) {
        warn!("channel_plan: remove sniffer {sniffer_id} SD save failed: {e}");
    }
}
