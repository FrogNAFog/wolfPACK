//! WiFi probe recon capture: orchestrate sniffers, reassemble ESP-NOW frags, write pcapng to SD.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use esp_idf_sys as sys;
use log::{info, warn};

use crate::storage::{self, SdIoHandle};
use wardriver_proto::{
    self as proto, pack_recon_cmd, unpack_recon_wifi_frag, ReconCmdOp, ReconCmdV1, ReconJobKind,
    SnifferRole, RECON_WIFI_CHANNEL_LIST_REVISION, RECON_WIFI_FRAG_MAX_PAYLOAD,
};
use wardriver_shared::espnow;
use wardriver_stats as stats;

static RECON_START_REQ: AtomicBool = AtomicBool::new(false);

/// Relative path under [`storage::sd_data_root()`] for UI (e.g. `recon/wifiprobes/20260429T120000_probecapture.pcapng`).
static RECON_OUTPUT_REL: Mutex<String> = Mutex::new(String::new());

struct ReconPartial {
    frag_total: u8,
    mpdu_len: u16,
    buf: Vec<u8>,
    if_id: u32,
    rssi: i8,
    channel: u8,
    got: u32,
}

struct Active {
    session_id: u32,
    end_mono_ms: u64,
    dwell_ms: u16,
    control_ch: u8,
    control_boot_id: u32,
    if_by_sniffer: HashMap<u32, u32>,
    _meta_path: std::path::PathBuf,
    partial: HashMap<(u32, u32), ReconPartial>,
}

static ACTIVE: Mutex<Option<Active>> = Mutex::new(None);

const RECON_DURATION_MS: u32 = 60_000;
const RECON_DWELL_MS: u16 = 120;

/// Eligible WiFi sniffers for a recon capture: non-zero id, WiFi role, seen within
/// 120 s, non-zero mac, sorted by sniffer_id. `None` when empty (start request cleared).
fn collect_eligible_wifi_sniffers(now_ms: u64) -> Option<Vec<(u32, [u8; 6])>> {
    let snap = stats::snapshot_sniffers();
    let mut wifi: Vec<(u32, [u8; 6])> = Vec::new();
    for s in snap.iter() {
        if s.sniffer_id == 0 {
            continue;
        }
        if s.role != SnifferRole::Wifi as u8 {
            continue;
        }
        if now_ms.saturating_sub(s.last_seen_ms) > 120_000 {
            continue;
        }
        if s.mac == [0u8; 6] {
            continue;
        }
        wifi.push((s.sniffer_id, s.mac));
    }
    wifi.sort_by_key(|w| w.0);
    if wifi.is_empty() {
        warn!("recon: no eligible WiFi sniffers; clearing start request");
        RECON_START_REQ.store(false, Ordering::Release);
        return None;
    }
    Some(wifi)
}

/// Build the pcapng header (SHB + one IDB per sniffer) and open the recon pcap via RPC.
fn open_recon_pcap(
    sd_io: &SdIoHandle,
    pcap_path: std::path::PathBuf,
    meta_path: std::path::PathBuf,
    wifi: &[(u32, [u8; 6])],
) -> Result<()> {
    let mut initial = Vec::new();
    write_shb(&mut initial)?;
    let link = 127u16;
    let snap = 65535u32;
    for (sid, _) in wifi {
        write_idb(&mut initial, link, snap, &format!("sniffer_{sid}"))?;
    }
    sd_io.rpc_recon_open(pcap_path, meta_path, initial)?;
    Ok(())
}

/// Pack the `ReconCmdV1::Start` and fan it out to every selected sniffer (adding peers).
fn broadcast_recon_start(
    wifi: &[(u32, [u8; 6])],
    session_id: u32,
    end_ms: u64,
    control_ch: u8,
    control_boot_id: u32,
) {
    let cmd = ReconCmdV1 {
        op: ReconCmdOp::Start,
        job_kind: ReconJobKind::WifiProbePcap,
        session_id,
        duration_ms: RECON_DURATION_MS,
        end_mono_ms: end_ms,
        dwell_ms: RECON_DWELL_MS,
        channel_list_revision: RECON_WIFI_CHANNEL_LIST_REVISION,
        phase_offset: 0,
        control_channel: control_ch.max(1),
        control_boot_id,
    };
    let payload = pack_recon_cmd(cmd);
    let peer_if = esp_idf_sys::wifi_interface_t_WIFI_IF_AP;
    for (_, mac) in wifi {
        let _ = espnow::add_peer(*mac, control_ch, peer_if);
        if let Err(e) = espnow::send(*mac, &payload) {
            warn!("recon: send start to {:02x?}: {e:#}", mac);
        }
    }
}

/// Start recon if requested: picks WiFi sniffers, opens pcapng, fans out [`ReconCmd`].
pub fn try_begin_pending_session(
    sd_io: &SdIoHandle,
    sniffers_mac: &std::sync::Arc<std::sync::Mutex<std::collections::HashSet<[u8; 6]>>>,
    control_boot_id: u32,
    control_ch: u8,
) -> Result<()> {
    if !RECON_START_REQ.load(Ordering::Acquire) {
        return Ok(());
    }
    if ACTIVE.lock().map(|g| g.is_some()).unwrap_or(false) {
        return Ok(());
    }
    if !stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        return Ok(());
    }

    let now_ms = stats::now_ms();
    let end_ms = now_ms.saturating_add(u64::from(RECON_DURATION_MS));
    let ts = fat_timestamp_utc_now();
    let dir = storage::recon_wifiprobes_dir();
    let _ = sd_io.submit_and_wait(storage::SdIoRpcOp::CreateDirs(vec![dir.clone()]));
    let fname = format!("{ts}_probecapture.pcapng");
    let pcap_path = dir.join(&fname);
    let meta_path = dir.join(format!("{ts}_probecapture.meta.json"));

    let Some(wifi) = collect_eligible_wifi_sniffers(now_ms) else {
        return Ok(());
    };
    if !take_start_request() {
        return Ok(());
    }

    let session_id: u32 = (unsafe { sys::esp_random() } & 0x7fff_ffff).max(1);
    let mut if_by = HashMap::new();
    for (i, (sid, _)) in wifi.iter().enumerate() {
        if_by.insert(*sid, i as u32);
    }

    open_recon_pcap(sd_io, pcap_path.clone(), meta_path.clone(), &wifi)?;

    let rel = format!("recon/wifiprobes/{fname}",);
    if let Ok(mut g) = RECON_OUTPUT_REL.lock() {
        *g = rel.clone();
    }

    broadcast_recon_start(&wifi, session_id, end_ms, control_ch, control_boot_id);
    if let Ok(mut g) = sniffers_mac.lock() {
        for (_, mac) in &wifi {
            g.insert(*mac);
        }
    }

    stats::set_recon_hold(true);
    stats::RECON_ACTIVE.store(1, Ordering::Release);
    stats::RECON_SESSION_ID.store(session_id, Ordering::Release);
    stats::set_recon_timing_ms(end_ms, RECON_DURATION_MS);

    let active = Active {
        session_id,
        end_mono_ms: end_ms,
        dwell_ms: RECON_DWELL_MS,
        control_ch: control_ch.max(1),
        control_boot_id,
        if_by_sniffer: if_by,
        _meta_path: meta_path,
        partial: HashMap::new(),
    };
    if let Ok(mut g) = ACTIVE.lock() {
        *g = Some(active);
    }

    info!(
        "recon: started session_id={} sniffers={} file={}",
        session_id,
        wifi.len(),
        fname
    );
    Ok(())
}

/// User / web requested a WiFi probe recon capture (control main loop will start when SD is ready).
pub fn request_wifi_probe_recon() {
    RECON_START_REQ.store(true, Ordering::Release);
}

pub fn take_start_request() -> bool {
    RECON_START_REQ.swap(false, Ordering::AcqRel)
}

pub fn output_relpath_for_api() -> String {
    RECON_OUTPUT_REL
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn wifi_channel_to_freq_mhz(ch: u8) -> u16 {
    match ch {
        1..=13 => 2407u16 + u16::from(ch) * 5,
        14 => 2484,
        32..=180 => 5000u16 + u16::from(ch) * 5,
        _ => 2412,
    }
}

/// Linux `ieee80211_radiotap.h` / [radiotap Channel](https://www.radiotap.org/fields/Channel.html)
/// flags (same bitmask Wireshark uses). Must match the 2.4 vs 5 GHz split in [`wifi_channel_to_freq_mhz`].
fn wifi_channel_radiotap_flags(ch: u8) -> u16 {
    const IEEE80211_CHAN_CCK: u16 = 0x0020;
    const IEEE80211_CHAN_OFDM: u16 = 0x0040;
    const IEEE80211_CHAN_2GHZ: u16 = 0x0080;
    const IEEE80211_CHAN_5GHZ: u16 = 0x0100;
    match ch {
        1..=14 => IEEE80211_CHAN_2GHZ | IEEE80211_CHAN_CCK | IEEE80211_CHAN_OFDM,
        32..=180 => IEEE80211_CHAN_5GHZ | IEEE80211_CHAN_OFDM,
        _ => IEEE80211_CHAN_2GHZ | IEEE80211_CHAN_CCK | IEEE80211_CHAN_OFDM,
    }
}

/// Minimal radiotap: TSFT=0, flags=0, Rate=0, Channel (freq+flags), dBm antenna signal.
fn radiotap_prefix(rssi: i8, ch: u8) -> [u8; 18] {
    let freq = wifi_channel_to_freq_mhz(ch).to_le_bytes();
    let ch_flags = wifi_channel_radiotap_flags(ch).to_le_bytes();
    let mut rt = [0u8; 18];
    rt[0] = 0;
    rt[1] = 0;
    rt[2] = 18;
    rt[3] = 0;
    // present: CHANNEL | DBM_ANTSIGNAL = 0x08 | 0x20 = 0x28
    rt[4] = 0x28;
    rt[5] = 0;
    rt[6] = 0;
    rt[7] = 0;
    rt[8] = freq[0];
    rt[9] = freq[1];
    rt[10] = ch_flags[0];
    rt[11] = ch_flags[1];
    rt[12] = rssi as u8;
    rt[13] = 0;
    rt[14] = 0;
    rt[15] = 0;
    rt[16] = 0;
    rt[17] = 0;
    rt
}

fn pad_to_4(n: usize) -> usize {
    (4 - (n % 4)) % 4
}

fn write_block(out: &mut Vec<u8>, block_type: u32, body: &[u8]) -> Result<()> {
    let pad = pad_to_4(body.len());
    let total = (12 + body.len() + pad) as u32;
    out.extend_from_slice(&block_type.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(body);
    for _ in 0..pad {
        out.push(0);
    }
    out.extend_from_slice(&total.to_le_bytes());
    Ok(())
}

fn write_shb(out: &mut Vec<u8>) -> Result<()> {
    let mut body = Vec::with_capacity(16);
    body.extend_from_slice(&0x1a2b3c4du32.to_le_bytes());
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&(-1i64).to_le_bytes());
    write_block(out, 0x0a0d0d0a, &body)
}

fn write_idb(out: &mut Vec<u8>, linktype: u16, snaplen: u32, if_name: &str) -> Result<()> {
    let mut body = Vec::new();
    body.extend_from_slice(&linktype.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&snaplen.to_le_bytes());
    if !if_name.is_empty() {
        let opt: u16 = 2;
        body.extend_from_slice(&opt.to_le_bytes());
        let nl = if_name.len() as u16;
        body.extend_from_slice(&nl.to_le_bytes());
        body.extend_from_slice(if_name.as_bytes());
        let p = pad_to_4(body.len());
        for _ in 0..p {
            body.push(0);
        }
    }
    body.extend_from_slice(&0u16.to_le_bytes());
    write_block(out, 0x00000001, &body)
}

fn write_epb_bytes(if_id: u32, ts_usec: u64, pkt: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut body = Vec::with_capacity(32 + pkt.len() + 4);
    body.extend_from_slice(&if_id.to_le_bytes());
    let hi = (ts_usec >> 32) as u32;
    let lo = (ts_usec & 0xffff_ffff) as u32;
    body.extend_from_slice(&hi.to_le_bytes());
    body.extend_from_slice(&lo.to_le_bytes());
    let cap = pkt.len() as u32;
    body.extend_from_slice(&cap.to_le_bytes());
    body.extend_from_slice(&cap.to_le_bytes());
    body.extend_from_slice(pkt);
    let p = pad_to_4(pkt.len());
    for _ in 0..p {
        body.push(0);
    }
    write_block(&mut out, 0x00000006, &body)?;
    Ok(out)
}

fn fat_timestamp_utc_now() -> String {
    let mut tv = sys::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        sys::gettimeofday(&mut tv, core::ptr::null_mut());
    }
    if tv.tv_sec <= 0 {
        return format!("boot{}", stats::now_ms());
    }
    let t = time_gmtime(tv.tv_sec);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        t.tm_year + 1900,
        t.tm_mon + 1,
        t.tm_mday,
        t.tm_hour,
        t.tm_min,
        t.tm_sec
    )
}

struct TmParts {
    tm_year: i32,
    tm_mon: i32,
    tm_mday: i32,
    tm_hour: i32,
    tm_min: i32,
    tm_sec: i32,
}

fn time_gmtime(sec: sys::time_t) -> TmParts {
    let mut t = sys::tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 0,
        tm_mon: 0,
        tm_year: 0,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: 0,
    };
    unsafe {
        sys::gmtime_r(&sec, &mut t);
    }
    TmParts {
        tm_year: t.tm_year.into(),
        tm_mon: t.tm_mon.into(),
        tm_mday: t.tm_mday.into(),
        tm_hour: t.tm_hour.into(),
        tm_min: t.tm_min.into(),
        tm_sec: t.tm_sec.into(),
    }
}

/// Drain completed recon session: close pcap, clear hold, send abort (idempotent).
pub fn poll_session_end(
    sd_io: &SdIoHandle,
    sniffers_mac: &std::sync::Arc<std::sync::Mutex<std::collections::HashSet<[u8; 6]>>>,
) {
    let now = stats::now_ms();
    let mut end = false;
    if let Ok(mut g) = ACTIVE.lock() {
        if let Some(a) = g.as_mut() {
            if now >= a.end_mono_ms.saturating_add(2_000) {
                end = true;
            }
        }
        if end {
            if let Some(a) = g.take() {
                let sniffers: Vec<[u8; 6]> = sniffers_mac
                    .lock()
                    .ok()
                    .map(|m| m.iter().copied().collect())
                    .unwrap_or_default();
                let abort = pack_recon_cmd(ReconCmdV1 {
                    op: ReconCmdOp::Abort,
                    job_kind: ReconJobKind::WifiProbePcap,
                    session_id: a.session_id,
                    duration_ms: 0,
                    end_mono_ms: 0,
                    dwell_ms: a.dwell_ms,
                    channel_list_revision: RECON_WIFI_CHANNEL_LIST_REVISION,
                    phase_offset: 0,
                    control_channel: a.control_ch,
                    control_boot_id: a.control_boot_id,
                });
                let peer_if = esp_idf_sys::wifi_interface_t_WIFI_IF_AP;
                for mac in sniffers {
                    let _ = espnow::add_peer(mac, a.control_ch, peer_if);
                    let _ = espnow::send(mac, &abort);
                }
                let n_if = a.if_by_sniffer.len();
                let n_pkt = stats::RECON_PKTS_WRITTEN.load(Ordering::Relaxed);
                let n_rx = stats::RECON_FRAGS_RX.load(Ordering::Relaxed);
                let meta = format!(
                    "{{\"proto_version\":{},\"session_id\":{},\"interfaces\":{},\"recon_frags_rx\":{},\"pcap_pkts\":{},\"channel_list_revision\":{}}}\n",
                    proto::PROTO_VERSION,
                    a.session_id,
                    n_if,
                    n_rx,
                    n_pkt,
                    RECON_WIFI_CHANNEL_LIST_REVISION
                );
                let _ = sd_io.rpc_recon_close(meta);
                info!("recon: session {} closed (pcap closed)", a.session_id);
            }
        }
    }
    if end {
        stats::set_recon_hold(false);
        stats::RECON_ACTIVE.store(0, Ordering::Release);
        stats::clear_recon_timing_ms();
        if let Ok(mut g) = RECON_OUTPUT_REL.lock() {
            g.clear();
        }
    }
}

/// Process one [`FrameType::ReconWifiFrag`] payload (already verified PROTO_VERSION).
pub fn ingest_wifi_frag(
    sd_io: &SdIoHandle,
    data: &[u8],
    last_sniffer_boot_id: &std::collections::HashMap<u32, u32>,
) -> Result<()> {
    let (hdr, chunk) = unpack_recon_wifi_frag(data)?;
    if hdr.frag_total == 0 || hdr.frag_total > 31 {
        stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    stats::RECON_FRAGS_RX.fetch_add(1, Ordering::Relaxed);

    let mut guard = ACTIVE
        .lock()
        .map_err(|_| anyhow!("recon active mutex poison"))?;
    let Some(act) = guard.as_mut() else {
        return Ok(());
    };
    if hdr.session_id != act.session_id {
        return Ok(());
    }
    if let Some(&boot) = last_sniffer_boot_id.get(&hdr.sniffer_id) {
        if boot != hdr.sniffer_boot_id {
            return Ok(());
        }
    }
    let if_id = *act
        .if_by_sniffer
        .get(&hdr.sniffer_id)
        .ok_or_else(|| anyhow!("recon unknown sniffer"))?;

    let key = (hdr.sniffer_id, hdr.capture_seq);
    let maxc = RECON_WIFI_FRAG_MAX_PAYLOAD;
    let off = (hdr.frag_idx as usize).saturating_mul(maxc);

    match act.partial.entry(key) {
        std::collections::hash_map::Entry::Vacant(v) => {
            if hdr.frag_idx != 0 {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            let mut buf = vec![0u8; hdr.mpdu_total_len as usize];
            if off + chunk.len() > buf.len() {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            buf[off..off + chunk.len()].copy_from_slice(chunk);
            let mask = 1u32 << hdr.frag_idx;
            v.insert(ReconPartial {
                frag_total: hdr.frag_total,
                mpdu_len: hdr.mpdu_total_len,
                buf,
                if_id,
                rssi: hdr.rssi,
                channel: hdr.channel,
                got: mask,
            });
        }
        std::collections::hash_map::Entry::Occupied(mut o) => {
            let p = o.get_mut();
            if p.frag_total != hdr.frag_total || p.mpdu_len != hdr.mpdu_total_len {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                o.remove();
                return Ok(());
            }
            if off + chunk.len() > p.buf.len() {
                stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
                o.remove();
                return Ok(());
            }
            p.buf[off..off + chunk.len()].copy_from_slice(chunk);
            p.got |= 1u32 << hdr.frag_idx;
        }
    }

    let done = if let Some(p) = act.partial.get(&key) {
        let need = (1u32 << p.frag_total) - 1;
        (p.got & need) == need
    } else {
        false
    };
    if done {
        let p = act
            .partial
            .remove(&key)
            .ok_or_else(|| anyhow!("recon partial vanished"))?;
        let rt = radiotap_prefix(p.rssi, p.channel);
        let mut pkt = Vec::with_capacity(rt.len() + p.buf.len());
        pkt.extend_from_slice(&rt);
        pkt.extend_from_slice(&p.buf);
        let ts_usec = stats::now_ms().saturating_mul(1000);
        let epb = write_epb_bytes(p.if_id, ts_usec, &pkt)?;
        if !sd_io.try_enqueue_recon_chunk(epb) {
            stats::note_recon_sd_chunk_drop();
        } else {
            stats::RECON_PKTS_WRITTEN.fetch_add(1, Ordering::Relaxed);
        }
    }

    if act.partial.len() > 256 {
        act.partial.clear();
        stats::RECON_REASM_FAIL.fetch_add(1, Ordering::Relaxed);
    }

    Ok(())
}

pub fn catalog_json() -> String {
    r#"[{"id":"wifi_probe_pcap","title":"WiFi probe requests (60s)","description":"2.4+5 GHz incl. DFS; merged pcapng on SD; ESP-NOW frags flush on control channel.","default_duration_sec":60,"wifi_only":true}]"#.to_string()
}
