//! Deferred NVS for PackLink TOFU from the TCP session pthread.
//!
//! `gw-packlink-tcp` runs on a PSRAM stack; NVS open/write and frame packing are fragile
//! there. The TCP thread enqueues trust ops and polls for pre-built wire replies; the
//! gateway main loop drains NVS on the internal-RAM stack and packs `TrustOk`/`TrustReject`.

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::Duration;

use wardriver_packlink::{pack_trust_ok, pack_trust_reject, trust_reject_reason, TrustRejectV1};
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::packlink_trust::{verify_phone_hello_outcome, VerifyPhoneHelloOutcome};

/// Default wait for the PackLink TCP thread calling [`PacklinkNvsDeferQueue::submit_verify_phone_hello`].
pub const PACKLINK_NVS_DEFER_WAIT_MS: u64 = 15_000;

const TOFU_HANDSHAKE_SEQ: u32 = 1;

/// Pre-packed handshake frame built on the main loop stack.
#[derive(Debug, Clone)]
pub enum PacklinkHandshakeReply {
    TrustOk(Vec<u8>),
    TrustReject(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum PacklinkNvsOp {
    VerifyPhoneHello { phone_id: String },
}

pub struct PacklinkNvsDeferRequest {
    pub op: PacklinkNvsOp,
    pub reply: SyncSender<Result<PacklinkHandshakeReply, String>>,
}

#[derive(Clone)]
pub struct PacklinkNvsDeferQueue {
    tx: SyncSender<PacklinkNvsDeferRequest>,
}

impl PacklinkNvsDeferQueue {
    pub fn pair(capacity: usize) -> (Self, Receiver<PacklinkNvsDeferRequest>) {
        let (tx, rx) = sync_channel(capacity);
        (Self { tx }, rx)
    }

    pub fn submit_verify_phone_hello(
        &self,
        phone_id: &str,
        timeout: Duration,
    ) -> Result<PacklinkHandshakeReply, String> {
        self.submit_and_wait(
            PacklinkNvsOp::VerifyPhoneHello {
                phone_id: phone_id.to_string(),
            },
            timeout,
        )
    }

    fn submit_and_wait(
        &self,
        op: PacklinkNvsOp,
        timeout: Duration,
    ) -> Result<PacklinkHandshakeReply, String> {
        let (reply_tx, reply_rx) = sync_channel(1);
        let req = PacklinkNvsDeferRequest {
            op,
            reply: reply_tx,
        };
        match self.tx.try_send(req) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Err("packlink NVS defer queue full".to_string());
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err("packlink NVS defer queue disconnected".to_string());
            }
        }
        let deadline = stats::now_ms().saturating_add(timeout.as_millis() as u64);
        loop {
            match reply_rx.try_recv() {
                Ok(r) => return r,
                Err(TryRecvError::Disconnected) => {
                    return Err("packlink NVS defer reply disconnected".to_string());
                }
                Err(TryRecvError::Empty) => {}
            }
            if stats::now_ms() > deadline {
                return Err("packlink NVS defer request timed out".to_string());
            }
            task_wdt::sleep_feeding_wdt(Duration::from_millis(5));
            task_wdt::reset_if_subscribed();
        }
    }
}

/// Drain pending PackLink NVS ops (call from gateway main loop only).
pub fn drain(rx: &Receiver<PacklinkNvsDeferRequest>) {
    loop {
        let req = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => break,
        };
        let result = run_packlink_nvs_op(req.op);
        let _ = req.reply.send(result);
    }
}

fn run_packlink_nvs_op(op: PacklinkNvsOp) -> Result<PacklinkHandshakeReply, String> {
    match op {
        PacklinkNvsOp::VerifyPhoneHello { phone_id } => {
            match verify_phone_hello_outcome(&phone_id)? {
                VerifyPhoneHelloOutcome::Accepted => Ok(PacklinkHandshakeReply::TrustOk(
                    pack_trust_ok(TOFU_HANDSHAKE_SEQ),
                )),
                VerifyPhoneHelloOutcome::PhoneMismatch => {
                    let reject = TrustRejectV1 {
                        reason: trust_reject_reason::PHONE_MISMATCH,
                    };
                    Ok(PacklinkHandshakeReply::TrustReject(pack_trust_reject(
                        &reject,
                        TOFU_HANDSHAKE_SEQ,
                    )))
                }
            }
        }
    }
}
