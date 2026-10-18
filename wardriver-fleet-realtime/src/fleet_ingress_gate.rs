//! Outer ingress roster gate shared by control node and BLE gateway.
//!
//! Pre-dispatch policy matches the control firmware: only discovery (`SnifferHello`)
//! and wolfPACK dump visit (`PeerJoin`) bypass the roster gate; handlers re-check
//! adoption where needed (e.g. `PeerJoin` after dispatch).

use wardriver_proto::FrameType;

/// True for frame types that bypass the outer roster gate before dispatch.
///
/// Control-parity policy: only [`FrameType::SnifferHello`] and [`FrameType::PeerJoin`]
/// skip the gate. `SnifferHello` is discovery; `PeerJoin` is handled downstream with
/// its own roster check. Every other uplink frame type must pass
/// [`ingress_roster_should_drop`] with a matching adopted `(sniffer_id, mac)`.
#[must_use]
pub fn bypass_roster_gate(frame_type: u8) -> bool {
    matches!(
        frame_type,
        x if x == FrameType::SnifferHello as u8 || x == FrameType::PeerJoin as u8
    )
}

/// True when the outer roster gate should drop the frame before dispatch.
///
/// Control-parity policy:
/// - `roster_allows` → never drop (adopted `(sniffer_id, mac)` on roster).
/// - [`bypass_roster_gate`] → never drop (`SnifferHello` / `PeerJoin` only).
/// - Otherwise → drop (peer not roster-authorized for this frame type).
#[must_use]
pub fn ingress_roster_should_drop(frame_type: u8, roster_allows: bool) -> bool {
    if roster_allows {
        return false;
    }
    if bypass_roster_gate(frame_type) {
        return false;
    }
    true
}
