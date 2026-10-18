//! SSID watch alert (`FrameType::SsidWatchAlert`) ESP-NOW handler.

use wardriver_proto::{self as proto, SsidWatchAlertV1};
use wardriver_stats as stats;
use wardriver_stats::SsidWatchLast;

use super::ingress::sync_adoption_from_roster;
use super::ingress_ctx::ControlIngressCtx;

pub(super) fn handle_ssid_watch_alert(ctx: &mut ControlIngressCtx<'_>, peer: [u8; 6], data: &[u8]) {
    let Ok(alert) = proto::unpack_ssid_watch_alert(data) else {
        return;
    };
    stats::sniffer_seen_liveness(alert.sniffer_id, &peer);
    sync_adoption_from_roster(alert.sniffer_id, &peer);
    if let Some(boot) = ctx.last_sniffer_boot_id.get(&alert.sniffer_id).copied() {
        if boot != alert.sniffer_boot_id {
            return;
        }
    }
    handle_ssid_watch_alert_inner(ctx, &alert);
}

fn handle_ssid_watch_alert_inner(ctx: &mut ControlIngressCtx<'_>, alert: &SsidWatchAlertV1) {
    stats::record_ssid_watch_alert(SsidWatchLast {
        last_seen_ms: ctx.now_ms,
        sniffer_id: alert.sniffer_id,
        bssid: alert.bssid,
        channel: alert.channel,
        rssi: alert.rssi,
        ssid: alert.ssid.clone(),
    });
    // OLED subtitle shows the matched SSID (ASCII-only fonts on board-xiao).
    ctx.alert_hub.trigger(
        crate::alert_feedback::AlertKind::SsidWatch,
        Some(alert.ssid.as_str()),
    );
}
