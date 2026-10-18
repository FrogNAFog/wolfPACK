//! Phone → gateway channel plan sync (PackLink `ChannelPlanSync` / `ChannelPlanSyncAck`).

use anyhow::{anyhow, Result};

use crate::{pack_frame, MsgType};

/// Wire schema version for [`ChannelPlanSyncV1`].
pub const CHANNEL_PLAN_SYNC_SCHEMA: u8 = 1;

/// Max sniffers in manual band lists / pin table (matches `wardriver_core::channel_plan::MAX_PLAN_SNIFFERS`).
pub const CHANNEL_PLAN_MAX_SNIFFERS: usize = 16;

/// Max channels per band pool on wire (2.4 + 5 + DFS lists are stored separately).
pub const CHANNEL_PLAN_MAX_POOL: usize = 32;

/// `GatewayInfoV1` optional capability tail: dedicated channel-plan sync supported.
pub mod gateway_capabilities {
    pub const CHANNEL_PLAN_SYNC: u8 = 1 << 0;
}

pub mod band_mode {
    pub const AUTO_BALANCE: u8 = 0;
    pub const MANUAL: u8 = 1;
}

pub mod channel_plan_sync_ack_result {
    pub const OK: u8 = 0;
    pub const REJECTED: u8 = 1;
}

pub mod channel_plan_sync_ack_error {
    pub const NONE: u8 = 0;
    pub const PARSE: u8 = 1;
    pub const VALIDATION: u8 = 2;
    pub const APPLY: u8 = 3;
}

/// Phone → gateway operator channel plan (control-parity fields).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ChannelPlanSyncV1 {
    pub schema_version: u8,
    pub revision: u32,
    pub include_2_4_ghz: bool,
    pub include_5_ghz: bool,
    pub include_dfs: bool,
    pub dwell_ms: u16,
    pub band_mode: u8,
    pub channels_2_4: Vec<u8>,
    pub channels_5: Vec<u8>,
    pub channels_5_dfs: Vec<u8>,
    pub sniffers_2_4: Vec<u32>,
    pub sniffers_5: Vec<u32>,
    /// `(sniffer_id, pinned_channel)`; channel `0` means explicitly unpinned.
    pub pins: Vec<(u32, u8)>,
}

/// Gateway → phone ack for [`ChannelPlanSync`](MsgType::ChannelPlanSync).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ChannelPlanSyncAckV1 {
    pub plan_seq: u32,
    pub revision: u32,
    pub result: u8,
    pub error_code: u8,
    pub hop_cycle_ms: u32,
    pub unassigned_count: u16,
}

/// Optional per-row adoption progress tail appended after plan extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SnifferStatusAdoptExtV1 {
    pub adopt_progress: u8,
    pub adopt_reason: u8,
}

pub const SNIFFER_STATUS_ADOPT_EXT_VERSION: u8 = 2;
pub const SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN: usize = 2;

pub mod adopt_progress {
    pub const NONE: u8 = 0;
    pub const WAITING_POLICY: u8 = 1;
    pub const RETRYING_WELCOME: u8 = 2;
    pub const CONFIRMED: u8 = 3;
    pub const TIMED_OUT_HELLO_LIVE: u8 = 4;
    pub const TIMED_OUT_NO_HELLO: u8 = 5;
}

pub mod adopt_reason {
    pub const NONE: u8 = 0;
    pub const SEND_FAILED: u8 = 1;
    pub const POSSIBLE_EXISTING_BINDING: u8 = 2;
    pub const ROSTER_REMOVED: u8 = 3;
}

#[must_use]
pub fn pack_sniffer_status_adopt_ext(
    ext: &SnifferStatusAdoptExtV1,
) -> [u8; SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN] {
    [ext.adopt_progress, ext.adopt_reason]
}

pub fn unpack_sniffer_status_adopt_ext(bytes: &[u8]) -> Result<SnifferStatusAdoptExtV1> {
    if bytes.len() < SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN {
        return Err(anyhow!("sniffer_status adopt ext short"));
    }
    Ok(SnifferStatusAdoptExtV1 {
        adopt_progress: bytes[0],
        adopt_reason: bytes[1],
    })
}

/// Optional per-row extension appended after legacy [`SnifferStatusV1`] rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SnifferStatusPlanExtV1 {
    pub five_ghz_capable: u8,
    pub assigned_band: u8,
    pub pinned_channel: u8,
    pub assigned_chan_count: u8,
    /// First channels of the active bucket (UI preview).
    pub assigned_channels_preview: [u8; 4],
}

pub const SNIFFER_STATUS_EXT_VERSION: u8 = 1;
pub const SNIFFER_STATUS_EXT_WIRE_LEN: usize = 8;

fn push_pool(out: &mut Vec<u8>, pool: &[u8]) {
    let n = pool.len().min(CHANNEL_PLAN_MAX_POOL) as u8;
    out.push(n);
    out.extend_from_slice(&pool[..usize::from(n)]);
}

fn read_pool(payload: &[u8], off: &mut usize) -> Result<Vec<u8>> {
    if *off >= payload.len() {
        return Err(anyhow!("channel_plan_sync pool count truncated"));
    }
    let n = payload[*off] as usize;
    *off += 1;
    if *off + n > payload.len() {
        return Err(anyhow!("channel_plan_sync pool bytes truncated"));
    }
    let v = payload[*off..*off + n].to_vec();
    *off += n;
    Ok(v)
}

fn push_u32_list(out: &mut Vec<u8>, ids: &[u32]) {
    let n = ids.len().min(CHANNEL_PLAN_MAX_SNIFFERS) as u8;
    out.push(n);
    for id in ids.iter().take(CHANNEL_PLAN_MAX_SNIFFERS) {
        out.extend_from_slice(&id.to_le_bytes());
    }
}

fn read_u32_list(payload: &[u8], off: &mut usize) -> Result<Vec<u32>> {
    if *off >= payload.len() {
        return Err(anyhow!("channel_plan_sync sniffer list count truncated"));
    }
    let n = payload[*off] as usize;
    *off += 1;
    if n > CHANNEL_PLAN_MAX_SNIFFERS {
        return Err(anyhow!("channel_plan_sync sniffer list too long"));
    }
    if *off + n * 4 > payload.len() {
        return Err(anyhow!("channel_plan_sync sniffer list truncated"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(u32::from_le_bytes(payload[*off..*off + 4].try_into()?));
        *off += 4;
    }
    Ok(out)
}

fn push_pins(out: &mut Vec<u8>, pins: &[(u32, u8)]) {
    let n = pins.len().min(CHANNEL_PLAN_MAX_SNIFFERS) as u8;
    out.push(n);
    for (id, ch) in pins.iter().take(CHANNEL_PLAN_MAX_SNIFFERS) {
        out.extend_from_slice(&id.to_le_bytes());
        out.push(*ch);
    }
}

fn read_pins(payload: &[u8], off: &mut usize) -> Result<Vec<(u32, u8)>> {
    if *off >= payload.len() {
        return Err(anyhow!("channel_plan_sync pin count truncated"));
    }
    let n = payload[*off] as usize;
    *off += 1;
    if n > CHANNEL_PLAN_MAX_SNIFFERS {
        return Err(anyhow!("channel_plan_sync pin table too long"));
    }
    if *off + n * 5 > payload.len() {
        return Err(anyhow!("channel_plan_sync pin table truncated"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let id = u32::from_le_bytes(payload[*off..*off + 4].try_into()?);
        let ch = payload[*off + 4];
        out.push((id, ch));
        *off += 5;
    }
    Ok(out)
}

pub fn pack_channel_plan_sync(plan: &ChannelPlanSyncV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::new();
    p.push(plan.schema_version.max(CHANNEL_PLAN_SYNC_SCHEMA));
    p.extend_from_slice(&plan.revision.to_le_bytes());
    let mut flags = 0u8;
    if plan.include_2_4_ghz {
        flags |= 1;
    }
    if plan.include_5_ghz {
        flags |= 2;
    }
    if plan.include_dfs {
        flags |= 4;
    }
    if plan.band_mode == band_mode::MANUAL {
        flags |= 8;
    }
    p.push(flags);
    p.extend_from_slice(&plan.dwell_ms.to_le_bytes());
    push_pool(&mut p, &plan.channels_2_4);
    push_pool(&mut p, &plan.channels_5);
    push_pool(&mut p, &plan.channels_5_dfs);
    push_u32_list(&mut p, &plan.sniffers_2_4);
    push_u32_list(&mut p, &plan.sniffers_5);
    push_pins(&mut p, &plan.pins);
    pack_frame(MsgType::ChannelPlanSync, 0, seq, &p)
}

pub fn unpack_channel_plan_sync(payload: &[u8]) -> Result<ChannelPlanSyncV1> {
    if payload.len() < 8 {
        return Err(anyhow!("channel_plan_sync payload short"));
    }
    let schema_version = payload[0];
    if schema_version != CHANNEL_PLAN_SYNC_SCHEMA {
        return Err(anyhow!(
            "unsupported channel_plan_sync schema {schema_version}"
        ));
    }
    let revision = u32::from_le_bytes(payload[1..5].try_into()?);
    let flags = payload[5];
    let dwell_ms = u16::from_le_bytes([payload[6], payload[7]]);
    let mut off = 8usize;
    let channels_2_4 = read_pool(payload, &mut off)?;
    let channels_5 = read_pool(payload, &mut off)?;
    let channels_5_dfs = read_pool(payload, &mut off)?;
    let sniffers_2_4 = read_u32_list(payload, &mut off)?;
    let sniffers_5 = read_u32_list(payload, &mut off)?;
    let pins = read_pins(payload, &mut off)?;
    Ok(ChannelPlanSyncV1 {
        schema_version,
        revision,
        include_2_4_ghz: flags & 1 != 0,
        include_5_ghz: flags & 2 != 0,
        include_dfs: flags & 4 != 0,
        dwell_ms,
        band_mode: if flags & 8 != 0 {
            band_mode::MANUAL
        } else {
            band_mode::AUTO_BALANCE
        },
        channels_2_4,
        channels_5,
        channels_5_dfs,
        sniffers_2_4,
        sniffers_5,
        pins,
    })
}

pub fn pack_channel_plan_sync_ack(ack: &ChannelPlanSyncAckV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(16);
    p.extend_from_slice(&ack.plan_seq.to_le_bytes());
    p.extend_from_slice(&ack.revision.to_le_bytes());
    p.push(ack.result);
    p.push(ack.error_code);
    p.extend_from_slice(&ack.hop_cycle_ms.to_le_bytes());
    p.extend_from_slice(&ack.unassigned_count.to_le_bytes());
    pack_frame(MsgType::ChannelPlanSyncAck, 0, seq, &p)
}

pub fn unpack_channel_plan_sync_ack(payload: &[u8]) -> Result<ChannelPlanSyncAckV1> {
    if payload.len() < 16 {
        return Err(anyhow!("channel_plan_sync_ack payload short"));
    }
    Ok(ChannelPlanSyncAckV1 {
        plan_seq: u32::from_le_bytes(payload[0..4].try_into()?),
        revision: u32::from_le_bytes(payload[4..8].try_into()?),
        result: payload[8],
        error_code: payload[9],
        hop_cycle_ms: u32::from_le_bytes(payload[10..14].try_into()?),
        unassigned_count: u16::from_le_bytes([payload[14], payload[15]]),
    })
}

pub fn pack_sniffer_status_plan_ext(
    ext: &SnifferStatusPlanExtV1,
) -> [u8; SNIFFER_STATUS_EXT_WIRE_LEN] {
    let mut out = [0u8; SNIFFER_STATUS_EXT_WIRE_LEN];
    out[0] = ext.five_ghz_capable;
    out[1] = ext.assigned_band;
    out[2] = ext.pinned_channel;
    out[3] = ext.assigned_chan_count;
    out[4..8].copy_from_slice(&ext.assigned_channels_preview);
    out
}

pub fn unpack_sniffer_status_plan_ext(bytes: &[u8]) -> Result<SnifferStatusPlanExtV1> {
    if bytes.len() < SNIFFER_STATUS_EXT_WIRE_LEN {
        return Err(anyhow!("sniffer_status ext short"));
    }
    let mut preview = [0u8; 4];
    preview.copy_from_slice(&bytes[4..8]);
    Ok(SnifferStatusPlanExtV1 {
        five_ghz_capable: bytes[0],
        assigned_band: bytes[1],
        pinned_channel: bytes[2],
        assigned_chan_count: bytes[3],
        assigned_channels_preview: preview,
    })
}
