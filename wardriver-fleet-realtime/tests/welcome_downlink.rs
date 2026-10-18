//! Host coverage: canonical welcome bundle stage ordering via mock hooks.

use std::cell::RefCell;

use anyhow::{anyhow, Result};
use wardriver_fleet_realtime::welcome_bundle::{
    run_welcome_bundle, WelcomeAttemptResult, WelcomeBundleHooks, WelcomeStage, WELCOME_STAGE_ORDER,
};

struct MockHooks {
    log: RefCell<Vec<WelcomeStage>>,
    fail_at: Option<WelcomeStage>,
    ack_seq: u32,
}

impl MockHooks {
    fn new(ack_seq: u32) -> Self {
        Self {
            log: RefCell::new(Vec::new()),
            fail_at: None,
            ack_seq,
        }
    }

    fn failing_at(stage: WelcomeStage, ack_seq: u32) -> Self {
        Self {
            log: RefCell::new(Vec::new()),
            fail_at: Some(stage),
            ack_seq,
        }
    }

    fn record(&self, stage: WelcomeStage) -> Result<()> {
        self.log.borrow_mut().push(stage);
        if self.fail_at == Some(stage) {
            Err(anyhow!("injected failure at {stage:?}"))
        } else {
            Ok(())
        }
    }
}

impl WelcomeBundleHooks for MockHooks {
    fn ensure_peer(&self) -> Result<()> {
        self.record(WelcomeStage::EnsurePeer)
    }

    fn send_control_welcome(&self) -> Result<()> {
        self.record(WelcomeStage::ControlWelcome)
    }

    fn send_start_stop(&self) -> Result<u32> {
        self.record(WelcomeStage::StartStop)?;
        Ok(self.ack_seq)
    }

    fn send_channel_schedule(&self) -> Result<()> {
        self.record(WelcomeStage::ChannelSchedule)
    }

    fn send_bssid_denylist(&self) -> Result<()> {
        self.record(WelcomeStage::BssidDenylist)
    }

    fn send_ssid_denylist(&self) -> Result<()> {
        self.record(WelcomeStage::SsidDenylist)
    }

    fn send_ssid_watchlist(&self) -> Result<()> {
        self.record(WelcomeStage::SsidWatchlist)
    }

    fn send_set_role_if_pending(&self) -> Result<()> {
        self.record(WelcomeStage::SetRole)
    }
}

#[test]
fn run_welcome_bundle_executes_stages_in_canonical_order() {
    let hooks = MockHooks::new(77);
    let result = run_welcome_bundle(&hooks);

    assert_eq!(result, WelcomeAttemptResult::success(77));
    assert_eq!(*hooks.log.borrow(), WELCOME_STAGE_ORDER.to_vec());
}

#[test]
fn run_welcome_bundle_failure_before_start_stop_has_no_ack_seq() {
    let hooks = MockHooks::failing_at(WelcomeStage::ControlWelcome, 99);
    let result = run_welcome_bundle(&hooks);

    assert_eq!(
        result,
        WelcomeAttemptResult::failure(WelcomeStage::ControlWelcome)
    );
    assert_eq!(
        *hooks.log.borrow(),
        vec![WelcomeStage::EnsurePeer, WelcomeStage::ControlWelcome]
    );
}

#[test]
fn run_welcome_bundle_failure_after_start_stop_retains_ack_seq() {
    let hooks = MockHooks::failing_at(WelcomeStage::SsidWatchlist, 42);
    let result = run_welcome_bundle(&hooks);

    assert_eq!(
        result,
        WelcomeAttemptResult::failure_after_start_stop(WelcomeStage::SsidWatchlist, 42)
    );
    let log = hooks.log.borrow();
    assert_eq!(log.last().copied(), Some(WelcomeStage::SsidWatchlist));
    assert!(log.contains(&WelcomeStage::StartStop));
}
