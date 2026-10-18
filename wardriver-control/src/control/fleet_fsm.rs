//! Fleet orchestrator: explicit phase FSM for the control main loop.
//!
//! Phases mirror operator-visible modes (dashboard hold, wardrive, upload, recon).
//! Inputs are the existing stats flags set by prelude, recon, upload, and gate ticks.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use log::{info, warn};

use wardriver_stats as stats;

use super::fleet_cmd::{self, FleetCommandBuilder};
use super::main_loop_state::{CoreDeps, FleetDeps};
use wardriver_proto::ControlCmd;

/// High-level fleet mode (precedence: Upload > Management > Recon > Wardrive > Idle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FleetPhase {
    #[default]
    Idle = 0,
    Wardrive = 1,
    Management = 2,
    Upload = 3,
    Recon = 4,
}

impl FleetPhase {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Wardrive,
            2 => Self::Management,
            3 => Self::Upload,
            4 => Self::Recon,
            _ => Self::Idle,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Wardrive => "wardrive",
            Self::Management => "management",
            Self::Upload => "upload",
            Self::Recon => "capture",
        }
    }
}

#[must_use]
pub fn derive_phase(
    hold: bool,
    recon: bool,
    upload_active: bool,
    scan_allowed: bool,
) -> FleetPhase {
    if upload_active {
        FleetPhase::Upload
    } else if hold {
        FleetPhase::Management
    } else if recon {
        FleetPhase::Recon
    } else if scan_allowed {
        FleetPhase::Wardrive
    } else {
        FleetPhase::Idle
    }
}

#[must_use]
pub fn phase_from_stats(scan_allowed: bool) -> FleetPhase {
    derive_phase(
        stats::fleet_management_hold(),
        stats::recon_hold(),
        stats::upload_in_progress(),
        scan_allowed,
    )
}

/// Published fleet phase + last transition (for `/api/stats` and logs).
#[derive(Debug, Clone, Copy)]
pub struct FleetOrchestrator {
    pub phase: FleetPhase,
    pub prev_phase: FleetPhase,
    pub phase_entered_ms: u64,
    pub transitions: u32,
}

impl Default for FleetOrchestrator {
    fn default() -> Self {
        let phase = FleetPhase::Idle;
        Self {
            phase,
            prev_phase: phase,
            phase_entered_ms: 0,
            transitions: 0,
        }
    }
}

impl FleetOrchestrator {
    /// Recompute phase from live stats; publish to [`stats::set_fleet_phase`] and log transitions.
    pub fn sync_from_stats(&mut self, scan_allowed: bool) -> FleetPhase {
        let next = phase_from_stats(scan_allowed);
        let now_ms = stats::now_ms();
        if next != self.phase {
            info!(
                "fleet: phase {} -> {} (hold={} recon={} upload={} scan_allowed={})",
                self.phase.name(),
                next.name(),
                stats::fleet_management_hold() as u8,
                stats::recon_hold() as u8,
                stats::upload_in_progress() as u8,
                scan_allowed as u8,
            );
            self.prev_phase = self.phase;
            self.phase = next;
            self.phase_entered_ms = now_ms;
            self.transitions = self.transitions.saturating_add(1);
        }
        stats::set_fleet_phase_metrics(next.as_u8(), self.transitions, self.phase_entered_ms);
        stats::set_wardrive_active(next == FleetPhase::Wardrive);
        next
    }
}

/// Dashboard mode requests and manual upload (sets hold / scanning flags consumed by gates).
pub(crate) fn apply_dashboard_mode_requests(fleet: &mut FleetDeps, core: &mut CoreDeps<'_>) {
    use super::boot_setup;
    use crate::config;
    use crate::storage;

    match stats::take_mode_request() {
        stats::MODE_REQ_MANAGEMENT => {
            fleet.scanning_allowed.store(false, Ordering::Release);
            stats::set_fleet_management_hold(true);
            let cmd_builder = super::fleet_cmd::fleet_cmd_builder_from_plan();
            let _ = fleet_cmd::broadcast_cmd(
                &fleet.sniffers,
                &fleet.cmd_seq,
                ControlCmd::StopScanning,
                None,
                fleet.peer_if,
                fleet.control_boot_id,
                false,
                &cmd_builder,
                None,
                0,
            );
        }
        stats::MODE_REQ_WARDRIVE => {
            stats::set_fleet_management_hold(false);
            fleet.scanning_allowed.store(false, Ordering::Release);
        }
        _ => {}
    }

    if stats::take_upload_request() {
        if let Ok(cfg_u) = config::Config::load(core.nvs) {
            if cfg_u.provisioning_complete()
                && (cfg_u.wigle_configured() || cfg_u.wdgwars_configured())
            {
                let paths = storage::Paths::new();
                // Check pending before tearing down SoftAP for STA.
                match crate::uploader::pending_has_uploadable_file(core.sd_io.as_ref(), &paths) {
                    Ok(false) => {
                        info!("control: manual upload: no pending capture files; skipping");
                        core.alert_hub.show_oled_status("UPLOAD", Some("0 pending"));
                    }
                    Ok(true) => {
                        stats::set_fleet_management_hold(true);
                        let cmd_builder = super::fleet_cmd::fleet_cmd_builder_from_plan();
                        let _ = fleet_cmd::broadcast_cmd(
                            &fleet.sniffers,
                            &fleet.cmd_seq,
                            ControlCmd::StopScanning,
                            None,
                            fleet.peer_if,
                            fleet.control_boot_id,
                            false,
                            &cmd_builder,
                            None,
                            0,
                        );
                        stats::reset_upload_run_counters();
                        stats::set_upload_phase(stats::UPLOAD_PHASE_CONNECTING);
                        core.alert_hub
                            .show_oled_status("UPLOAD", Some("connecting..."));
                        match boot_setup::connect_sta_only(core.wifi, &cfg_u) {
                            Ok(()) => {
                                boot_setup::control_upload_pending(
                                    Arc::clone(&core.alert_hub),
                                    &cfg_u,
                                    &paths,
                                    Arc::clone(&core.sd_io),
                                    "control: manual upload",
                                    false,
                                );
                            }
                            Err(e) => {
                                warn!("control: manual upload: STA connect failed: {e:#}");
                                stats::set_upload_phase(stats::UPLOAD_PHASE_FAILED);
                                core.alert_hub.show_oled_status("UPLOAD", Some("no WiFi"));
                                boot_setup::restore_control_softap_home_channel(core.wifi, &cfg_u);
                            }
                        }
                        boot_setup::restore_control_softap_home_channel(core.wifi, &cfg_u);
                    }
                    Err(e) => {
                        warn!("control: manual upload: pending check: {e:#}");
                    }
                }
            } else {
                warn!("control: manual upload: skipped (no home WiFi or no upload creds in NVS)");
                core.alert_hub.show_oled_status("UPLOAD", Some("no keys"));
            }
        }
        stats::set_fleet_management_hold(false);
        fleet.scanning_allowed.store(false, Ordering::Release);
        stats::clear_upload_phase();
    }
}
