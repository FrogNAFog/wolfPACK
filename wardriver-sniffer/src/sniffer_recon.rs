//! Recon capture: channel hop cadence, promisc probe-request dwell, and DFS ESP-NOW TX queue.

use std::sync::atomic::Ordering;
use std::time::Duration;
use wardriver_shared::spiram_queue::Receiver as PromiscReceiver;

use wardriver_shared::spiram_queue::Receiver as EspnowRxReceiver;

use anyhow::Result;
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::warn;

use crate::sniffer_espnow::{SnifferCtx, RECON_SESSION};
use crate::sniffer_wifi::sniffer_boot_id;
use crate::sniffer_wifi::{
    recon_flush_dfs_tx_queue, recon_push_dfs_tx_payload, try_set_channel_with_recovery,
};
use crate::wifi_promisc::{self, PromiscRecord};
use wardriver_core::channel_plan::is_dfs_channel;
use wardriver_proto::{self as proto, SnifferState};
use wardriver_shared::espnow;
use wardriver_stats as stats;

const RECON_SLICE_MS: u16 = 40;

/// Per-session channel hop and capture sequencing (promisc handle lives in the main loop).
pub(crate) struct ReconChannelState {
    pub ch_idx: usize,
    pub elapsed_on_ch_ms: u32,
    pub capture_seq: u32,
}

impl ReconChannelState {
    pub fn new() -> Self {
        Self {
            ch_idx: 0,
            elapsed_on_ch_ms: 0,
            capture_seq: 1,
        }
    }

    pub fn reset(&mut self) {
        self.ch_idx = 0;
        self.elapsed_on_ch_ms = 0;
    }
}

/// One main-loop tick while in [`SnifferState::ReconCapturing`].
///
/// Returns `Ok(true)` when the caller should `continue` the main loop (still capturing or
/// just exited recon).
pub(crate) fn run_recon_tick<'a>(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    promisc_rx: &'a PromiscReceiver<PromiscRecord>,
    ctx: &mut SnifferCtx<'_>,
    rx_rx: &EspnowRxReceiver<espnow::RxFrame>,
    now_ms: u64,
    sniffer_id: u32,
    recon_cap: &mut Option<wifi_promisc::PromiscCapture<'a>>,
    recon: &mut ReconChannelState,
) -> Result<bool> {
    let params = match RECON_SESSION.lock().ok().and_then(|g| *g) {
        None => {
            if let (Some(mac), true) = (*ctx.control_mac, *ctx.control_established) {
                recon_flush_dfs_tx_queue(ctx.schedule.control_channel, mac);
            }
            *ctx.state = SnifferState::Idle;
            if let Some(c) = recon_cap.take() {
                let _ = c.end(wifi.wifi_mut());
            }
            let _ = espnow::set_channel(ctx.schedule.control_channel);
            return Ok(true);
        }
        Some(p) => p,
    };
    if now_ms >= params.end_mono_ms {
        if let (Some(mac), true) = (*ctx.control_mac, *ctx.control_established) {
            recon_flush_dfs_tx_queue(params.control_ch, mac);
        }
        let _ = RECON_SESSION.lock().map(|mut g| *g = None);
        *ctx.state = SnifferState::Idle;
        if let Some(c) = recon_cap.take() {
            let _ = c.end(wifi.wifi_mut());
        }
        let _ = espnow::set_channel(params.control_ch);
        return Ok(true);
    }
    if recon_cap.is_none() {
        match wifi_promisc::PromiscCapture::begin(
            wifi.wifi_mut(),
            promisc_rx,
            sniffer_id,
            ctx.cfg.detection_disable_wifi_mask,
            ctx.cfg.remote_id_disable_wifi_mask,
            0,
            crate::sniffer_wifi::sniffer_boot_id(),
            false,
        ) {
            Ok(c) => *recon_cap = Some(c),
            Err(e) => {
                warn!("sniffer: recon PromiscCapture::begin: {e:#}");
                let _ = RECON_SESSION.lock().map(|mut g| *g = None);
                *ctx.state = SnifferState::Idle;
                return Ok(true);
            }
        }
    }
    let cap = recon_cap.as_ref().unwrap();
    let n_ch = proto::RECON_WIFI_CHANNELS.len().max(1);
    let ch = proto::RECON_WIFI_CHANNELS[(recon.ch_idx + sniffer_id as usize) % n_ch].max(1);
    let dwell = params.dwell_ms as u32;
    if recon.elapsed_on_ch_ms == 0 {
        if recon.ch_idx > 0 {
            if let Some(mac) = *ctx.control_mac {
                recon_flush_dfs_tx_queue(params.control_ch, mac);
            }
        }
        if !try_set_channel_with_recovery(wifi, ch, "recon") {
            warn!("sniffer: recon hop ch={ch} failed");
        }
    }
    let slice = (RECON_SLICE_MS as u32).min(dwell.saturating_sub(recon.elapsed_on_ch_ms)) as u16;
    if slice == 0 {
        recon.ch_idx = (recon.ch_idx + 1) % n_ch;
        recon.elapsed_on_ch_ms = 0;
        return Ok(true);
    }
    let mut frags_tx: u32 = 0;
    {
        let mac_opt = *ctx.control_mac;
        let established = *ctx.control_established;
        cap.dwell_probe_requests(ch, slice, |pl, rssi, eff_ch| {
            if !established {
                return;
            }
            let Some(mac) = mac_opt else {
                return;
            };
            let total = pl.len() as u16;
            let cap_seq = recon.capture_seq;
            recon.capture_seq = recon.capture_seq.wrapping_add(1);
            let max_chunk = proto::RECON_WIFI_FRAG_MAX_PAYLOAD;
            let nt = ((pl.len() + max_chunk - 1) / max_chunk).max(1);
            if nt > u8::MAX as usize {
                return;
            }
            let nt_u8 = nt as u8;
            let mono_ts32 = stats::now_ms() as u32;
            for fi in 0..nt {
                let off = fi * max_chunk;
                let end = (off + max_chunk).min(pl.len());
                let chunk = &pl[off..end];
                let hdr = proto::ReconWifiFragV1Header {
                    session_id: params.session_id,
                    sniffer_id,
                    sniffer_boot_id: sniffer_boot_id(),
                    capture_seq: cap_seq,
                    frag_idx: fi as u8,
                    frag_total: nt_u8,
                    flags: 0,
                    mono_ts32,
                    rssi,
                    channel: eff_ch,
                    mpdu_total_len: total,
                };
                if let Ok(payload) = proto::pack_recon_wifi_frag(hdr, chunk) {
                    if is_dfs_channel(ch) {
                        recon_push_dfs_tx_payload(payload);
                    } else {
                        let _ = espnow::send(mac, &payload);
                        frags_tx = frags_tx.saturating_add(1);
                    }
                }
            }
        });
    }
    stats::RECON_FRAGS_TX_SNIFFER.fetch_add(frags_tx, Ordering::Relaxed);
    cap.extend_alerted_probe_srcs(ctx.alerted_probe_srcs);
    recon.elapsed_on_ch_ms += slice as u32;
    if recon.elapsed_on_ch_ms >= dwell {
        recon.ch_idx = (recon.ch_idx + 1) % n_ch;
        recon.elapsed_on_ch_ms = 0;
    }
    ctx.drain_rx(rx_rx)?;
    if *ctx.state != SnifferState::ReconCapturing {
        return Ok(true);
    }
    // Recon: do not send ACK while off-channel; defer to control-channel window.
    std::thread::sleep(Duration::from_millis(2));
    Ok(true)
}
