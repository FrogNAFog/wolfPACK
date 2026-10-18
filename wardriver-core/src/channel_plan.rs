//! Operator channel plan: bands, bucketing, dwell clamps, SD key/value encode/decode.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{anyhow, Result};
use wardriver_proto::CHANNELS_MAX;

/// Defensive upper bound on a single sniffer's bucket (wire array length).
pub const PER_NODE_CAP: usize = CHANNELS_MAX;

/// Default per-channel listen time (ms).
pub const DEFAULT_DWELL_MS: u16 = wardriver_proto::DEFAULT_SCAN_DWELL_MS;

/// Hard clamp on the operator-set dwell (matches PACK's 50..=10_000 range).
pub const MIN_DWELL_MS: u16 = 50;
pub const MAX_DWELL_MS: u16 = 10_000;

pub const MAX_PLAN_SNIFFERS: usize = 16;

#[must_use]
pub fn default_channels_2_4() -> Vec<u8> {
    (1..=11).collect()
}

#[must_use]
pub fn default_channels_5() -> Vec<u8> {
    vec![36, 40, 44, 48, 149, 153, 157, 161, 165, 169, 173, 177]
}

#[must_use]
pub fn default_channels_5_dfs() -> Vec<u8> {
    vec![
        52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
    ]
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PerSnifferChannel {
    pub sniffer_id: u32,
    pub pinned_channel: Option<u8>,
}

/// How sniffers are placed on 2.4 vs 5 GHz band lists before bucket partitioning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BandAssignmentMode {
    /// Rebuild band lists to keep per-sniffer hop buckets roughly balanced.
    #[default]
    AutoBalance,
    /// Use only `sniffers_2_4` / `sniffers_5` from the saved plan (after prune).
    Manual,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Band {
    TwoFour,
    Five,
}

impl Band {
    #[must_use]
    pub const fn as_wire(self) -> u8 {
        match self {
            Band::TwoFour => 1,
            Band::Five => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelPlan {
    pub include_2_4_ghz: bool,
    pub include_5_ghz: bool,
    pub include_dfs: bool,
    pub dwell_ms: u16,
    pub channels_2_4: Vec<u8>,
    pub channels_5: Vec<u8>,
    pub channels_5_dfs: Vec<u8>,
    pub sniffers_2_4: Vec<u32>,
    pub sniffers_5: Vec<u32>,
    pub per_sniffer: Vec<PerSnifferChannel>,
    pub band_mode: BandAssignmentMode,
}

impl Default for ChannelPlan {
    fn default() -> Self {
        Self {
            include_2_4_ghz: true,
            include_5_ghz: true,
            include_dfs: true,
            dwell_ms: DEFAULT_DWELL_MS,
            channels_2_4: default_channels_2_4(),
            channels_5: default_channels_5(),
            channels_5_dfs: default_channels_5_dfs(),
            sniffers_2_4: Vec::new(),
            sniffers_5: Vec::new(),
            per_sniffer: Vec::new(),
            band_mode: BandAssignmentMode::AutoBalance,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeSchedule {
    pub channels: [u8; CHANNELS_MAX],
    pub chan_count: u8,
    pub dwell_ms: u16,
    pub band: Option<Band>,
    pub pinned: bool,
}

impl NodeSchedule {
    #[must_use]
    pub const fn empty(dwell_ms: u16) -> Self {
        Self {
            channels: [0u8; CHANNELS_MAX],
            chan_count: 0,
            dwell_ms,
            band: None,
            pinned: false,
        }
    }

    fn from_bucket(bucket: &[u8], band: Option<Band>, dwell_ms: u16) -> Self {
        let mut channels = [0u8; CHANNELS_MAX];
        let count = bucket.len().min(CHANNELS_MAX);
        channels[..count].copy_from_slice(&bucket[..count]);
        Self {
            channels,
            chan_count: count as u8,
            dwell_ms,
            band,
            pinned: false,
        }
    }

    fn pinned(ch: u8, band: Band, dwell_ms: u16) -> Self {
        let mut channels = [0u8; CHANNELS_MAX];
        channels[0] = ch;
        Self {
            channels,
            chan_count: 1,
            dwell_ms,
            band: Some(band),
            pinned: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Effective {
    pub schedules: BTreeMap<u32, NodeSchedule>,
    pub hop_cycle_ms: u32,
    /// Channels that did not fit any sniffer bucket (auto mode, pool exceeds fleet capacity).
    pub unassigned: Vec<u8>,
}

/// Per-sniffer contiguous channel buckets; overflow channels are listed separately.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BalancedBuckets {
    pub buckets: BTreeMap<u32, Vec<u8>>,
    pub unassigned: Vec<u8>,
}

#[must_use]
pub fn is_2_4_channel(ch: u8) -> bool {
    (1..=14).contains(&ch)
}

#[must_use]
pub fn is_5ghz_channel(ch: u8) -> bool {
    matches!(
        ch,
        36 | 40
            | 44
            | 48
            | 52
            | 56
            | 60
            | 64
            | 100
            | 104
            | 108
            | 112
            | 116
            | 120
            | 124
            | 128
            | 132
            | 136
            | 140
            | 144
            | 149
            | 153
            | 157
            | 161
            | 165
            | 169
            | 173
            | 177
    )
}

#[must_use]
pub fn is_dfs_channel(ch: u8) -> bool {
    matches!(
        ch,
        52 | 56 | 60 | 64 | 100 | 104 | 108 | 112 | 116 | 120 | 124 | 128 | 132 | 136 | 140 | 144
    )
}

#[must_use]
pub fn is_known_wifi_channel(ch: u8) -> bool {
    ch != 0 && (is_2_4_channel(ch) || is_5ghz_channel(ch))
}

#[must_use]
pub fn normalize_wifi_channels(mut channels: Vec<u8>) -> Vec<u8> {
    channels.retain(|&c| is_known_wifi_channel(c));
    channels.sort_unstable();
    channels.dedup();
    channels
}

/// LPR cameras probe ascending; sniffers visit descending channel numbers for better rendezvous.
/// Sparse buckets like `[1, 6, 11]` must reverse the vec, not walk indices backward.
#[must_use]
pub fn wardrive_hop_channels(channels_asc: &[u8]) -> Vec<u8> {
    let mut v = channels_asc.to_vec();
    v.reverse();
    v
}

/// Split sorted `channels` into `n` adjacent slices; first `len % n` buckets get +1 channel.
#[must_use]
pub fn partition_channels_contiguously(channels: &[u8], n: usize) -> Vec<Vec<u8>> {
    if n == 0 || channels.is_empty() {
        return Vec::new();
    }
    let len = channels.len();
    let base = len / n;
    let rem = len % n;
    let mut out = Vec::with_capacity(n);
    let mut idx = 0usize;
    for i in 0..n {
        let size = base + usize::from(i < rem);
        if size == 0 {
            out.push(Vec::new());
        } else {
            out.push(channels[idx..idx + size].to_vec());
            idx += size;
        }
    }
    out
}

fn bucket_slice_with_cap(slice: &[u8]) -> (Vec<u8>, Vec<u8>) {
    if slice.len() <= PER_NODE_CAP {
        return (slice.to_vec(), Vec::new());
    }
    (
        slice[..PER_NODE_CAP].to_vec(),
        slice[PER_NODE_CAP..].to_vec(),
    )
}

impl ChannelPlan {
    pub fn normalize_channels(&mut self) {
        self.channels_2_4 = normalize_wifi_channels(std::mem::take(&mut self.channels_2_4));
        self.channels_5 = normalize_wifi_channels(std::mem::take(&mut self.channels_5));
        self.channels_5_dfs = normalize_wifi_channels(std::mem::take(&mut self.channels_5_dfs));
    }

    pub fn clamp_dwell(&mut self) {
        self.dwell_ms = self.dwell_ms.clamp(MIN_DWELL_MS, MAX_DWELL_MS);
    }

    pub fn prune_unknown_sniffers(&mut self, active_ids: &[u32]) {
        let active: HashSet<u32> = active_ids.iter().copied().collect();
        self.sniffers_2_4.retain(|id| active.contains(id));
        self.sniffers_5.retain(|id| active.contains(id));
        self.per_sniffer.retain(|p| active.contains(&p.sniffer_id));
    }

    #[must_use]
    pub fn master_scan_pool(&self) -> Vec<u8> {
        master_scan_pool(self)
    }

    #[must_use]
    pub fn bootstrap_bucket(&self) -> ([u8; CHANNELS_MAX], u8) {
        let mut out = [0u8; CHANNELS_MAX];
        let pick: &[u8] = if self.include_2_4_ghz && !self.channels_2_4.is_empty() {
            &self.channels_2_4
        } else if self.include_5_ghz && !self.channels_5.is_empty() {
            &self.channels_5
        } else if self.include_5_ghz && self.include_dfs && !self.channels_5_dfs.is_empty() {
            &self.channels_5_dfs
        } else {
            return (out, 0);
        };
        let n = pick.len().min(CHANNELS_MAX);
        out[..n].copy_from_slice(&pick[..n]);
        (out, n as u8)
    }

    #[must_use]
    pub fn band_pool(&self, band: Band) -> Vec<u8> {
        match band {
            Band::TwoFour => {
                if self.include_2_4_ghz {
                    self.channels_2_4.clone()
                } else {
                    Vec::new()
                }
            }
            Band::Five => self.five_ghz_pool(true),
        }
    }

    fn five_ghz_pool(&self, with_dfs: bool) -> Vec<u8> {
        if !self.include_5_ghz {
            return Vec::new();
        }
        let mut v = self.channels_5.clone();
        if with_dfs && self.include_dfs {
            v.extend_from_slice(&self.channels_5_dfs);
        }
        normalize_wifi_channels(v)
    }

    fn pinned_map(&self) -> HashMap<u32, u8> {
        self.per_sniffer
            .iter()
            .filter_map(|p| p.pinned_channel.map(|ch| (p.sniffer_id, ch)))
            .collect()
    }
}

/// Merged sorted unique channels from enabled 2.4 / 5 / DFS lists.
#[must_use]
pub fn master_scan_pool(plan: &ChannelPlan) -> Vec<u8> {
    let mut v = Vec::new();
    if plan.include_2_4_ghz {
        v.extend_from_slice(&plan.channels_2_4);
    }
    if plan.include_5_ghz {
        v.extend_from_slice(&plan.channels_5);
        if plan.include_dfs {
            v.extend_from_slice(&plan.channels_5_dfs);
        }
    }
    normalize_wifi_channels(v)
}

/// Contiguous slices of the master scan pool across active sniffers; each bucket capped at [`PER_NODE_CAP`].
#[must_use]
pub fn balance_channels_across_sniffers(
    plan: &ChannelPlan,
    active_ids_sorted: &[u32],
) -> BalancedBuckets {
    let pool = master_scan_pool(plan);
    let mut ids: Vec<u32> = active_ids_sorted.to_vec();
    ids.sort_unstable();
    ids.dedup();

    if ids.is_empty() {
        return BalancedBuckets {
            buckets: BTreeMap::new(),
            unassigned: pool,
        };
    }

    let slices = partition_channels_contiguously(&pool, ids.len());
    let mut buckets = BTreeMap::new();
    let mut unassigned = Vec::new();
    for (id, slice) in ids.into_iter().zip(slices) {
        let (bucket, overflow) = bucket_slice_with_cap(&slice);
        buckets.insert(id, bucket);
        unassigned.extend(overflow);
    }

    BalancedBuckets {
        buckets,
        unassigned,
    }
}

fn hopping_sniffers_on_band(
    plan: &ChannelPlan,
    band: Band,
    active_ids: &[u32],
    pinned: &HashMap<u32, u8>,
) -> Vec<u32> {
    let active: HashSet<u32> = active_ids.iter().copied().collect();
    let list = match band {
        Band::TwoFour => &plan.sniffers_2_4,
        Band::Five => &plan.sniffers_5,
    };
    let mut v: Vec<u32> = list
        .iter()
        .filter(|id| active.contains(id) && !pinned.contains_key(id))
        .copied()
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[must_use]
pub fn effective_from_plan(plan: &ChannelPlan, active_ids: &[u32]) -> Effective {
    let dwell = plan.dwell_ms.clamp(MIN_DWELL_MS, MAX_DWELL_MS);

    if plan.band_mode == BandAssignmentMode::AutoBalance {
        return effective_auto_balance(plan, active_ids, dwell);
    }

    let mut schedules: BTreeMap<u32, NodeSchedule> = BTreeMap::new();
    let pinned = plan.pinned_map();

    let band_of = |id: u32| -> Option<Band> {
        if plan.sniffers_2_4.contains(&id) {
            Some(Band::TwoFour)
        } else if plan.sniffers_5.contains(&id) {
            Some(Band::Five)
        } else {
            None
        }
    };

    for (id, ch) in pinned.iter() {
        if !active_ids.contains(id) {
            continue;
        }
        let band = band_of(*id).unwrap_or(Band::TwoFour);
        schedules.insert(*id, NodeSchedule::pinned(*ch, band, dwell));
    }

    let mut max_bucket = 0usize;

    for band in [Band::TwoFour, Band::Five] {
        let hop_ids = hopping_sniffers_on_band(plan, band, active_ids, &pinned);
        let pool = plan.band_pool(band);
        if hop_ids.is_empty() {
            // no-op
        } else if pool.is_empty() {
            for id in hop_ids {
                schedules.insert(id, NodeSchedule::empty(dwell));
            }
        } else {
            let buckets = partition_channels_contiguously(&pool, hop_ids.len());
            for (id, slice) in hop_ids.into_iter().zip(buckets) {
                let (bucket, _) = bucket_slice_with_cap(&slice);
                max_bucket = max_bucket.max(bucket.len());
                schedules.insert(id, NodeSchedule::from_bucket(&bucket, Some(band), dwell));
            }
        }
    }

    for id in active_ids {
        schedules
            .entry(*id)
            .or_insert_with(|| NodeSchedule::empty(dwell));
    }

    let hop_cycle_ms = (max_bucket as u32).saturating_mul(dwell as u32);
    Effective {
        schedules,
        hop_cycle_ms,
        unassigned: Vec::new(),
    }
}

fn effective_auto_balance(plan: &ChannelPlan, active_ids: &[u32], dwell: u16) -> Effective {
    let balanced = balance_channels_across_sniffers(plan, active_ids);
    let mut schedules: BTreeMap<u32, NodeSchedule> = BTreeMap::new();
    let mut max_bucket = 0usize;

    for (id, bucket) in balanced.buckets {
        if !active_ids.contains(&id) {
            continue;
        }
        max_bucket = max_bucket.max(bucket.len());
        schedules.insert(id, NodeSchedule::from_bucket(&bucket, None, dwell));
    }

    for id in active_ids {
        schedules
            .entry(*id)
            .or_insert_with(|| NodeSchedule::empty(dwell));
    }

    let hop_cycle_ms = (max_bucket as u32).saturating_mul(dwell as u32);
    Effective {
        schedules,
        hop_cycle_ms,
        unassigned: balanced.unassigned,
    }
}

const FILE_VERSION: u32 = 1;

fn join_csv<T: std::fmt::Display>(v: &[T]) -> String {
    v.iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_csv<T: std::str::FromStr>(s: &str) -> Vec<T> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<T>().ok())
        .collect()
}

impl ChannelPlan {
    #[must_use]
    pub fn encode_kv(&self) -> String {
        let mut out = String::new();
        out.push_str("# wolfPACK channel plan v1\n");
        out.push_str(&format!("version={FILE_VERSION}\n"));
        out.push_str(&format!("include_2_4={}\n", self.include_2_4_ghz as u8));
        out.push_str(&format!("include_5={}\n", self.include_5_ghz as u8));
        out.push_str(&format!("include_dfs={}\n", self.include_dfs as u8));
        out.push_str(&format!("dwell_ms={}\n", self.dwell_ms));
        out.push_str(&format!(
            "band_mode={}\n",
            match self.band_mode {
                BandAssignmentMode::AutoBalance => "auto",
                BandAssignmentMode::Manual => "manual",
            }
        ));
        out.push_str(&format!("channels_2_4={}\n", join_csv(&self.channels_2_4)));
        out.push_str(&format!("channels_5={}\n", join_csv(&self.channels_5)));
        out.push_str(&format!(
            "channels_5_dfs={}\n",
            join_csv(&self.channels_5_dfs)
        ));
        out.push_str(&format!("sniffers_2_4={}\n", join_csv(&self.sniffers_2_4)));
        out.push_str(&format!("sniffers_5={}\n", join_csv(&self.sniffers_5)));
        for p in &self.per_sniffer {
            if let Some(ch) = p.pinned_channel {
                out.push_str(&format!("pin={}:{}\n", p.sniffer_id, ch));
            }
        }
        out
    }

    pub fn decode_kv(text: &str) -> Result<Self> {
        let mut plan = ChannelPlan::default();
        plan.sniffers_2_4.clear();
        plan.sniffers_5.clear();
        plan.per_sniffer.clear();

        let mut saw_version = false;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };
            match key {
                "version" => {
                    let v: u32 = value
                        .parse()
                        .map_err(|_| anyhow!("invalid version: {value:?}"))?;
                    if v != FILE_VERSION {
                        return Err(anyhow!("unsupported channel_plan version {v}"));
                    }
                    saw_version = true;
                }
                "include_2_4" => plan.include_2_4_ghz = value != "0",
                "include_5" => plan.include_5_ghz = value != "0",
                "include_dfs" => plan.include_dfs = value != "0",
                "band_mode" => {
                    plan.band_mode = match value {
                        "manual" => BandAssignmentMode::Manual,
                        _ => BandAssignmentMode::AutoBalance,
                    };
                }
                "dwell_ms" => {
                    if let Ok(v) = value.parse::<u16>() {
                        plan.dwell_ms = v;
                    }
                }
                "channels_2_4" => plan.channels_2_4 = parse_csv::<u8>(value),
                "channels_5" => plan.channels_5 = parse_csv::<u8>(value),
                "channels_5_dfs" => plan.channels_5_dfs = parse_csv::<u8>(value),
                "sniffers_2_4" => plan.sniffers_2_4 = parse_csv::<u32>(value),
                "sniffers_5" => plan.sniffers_5 = parse_csv::<u32>(value),
                "pin" => {
                    if let Some((id_s, ch_s)) = value.split_once(':') {
                        if let (Ok(id), Ok(ch)) =
                            (id_s.trim().parse::<u32>(), ch_s.trim().parse::<u8>())
                        {
                            plan.per_sniffer.push(PerSnifferChannel {
                                sniffer_id: id,
                                pinned_channel: Some(ch),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        if !saw_version {
            return Err(anyhow!("channel_plan: missing version= line"));
        }
        plan.normalize_channels();
        plan.clamp_dwell();
        plan.sniffers_2_4.sort_unstable();
        plan.sniffers_2_4.dedup();
        plan.sniffers_5.sort_unstable();
        plan.sniffers_5.dedup();
        Ok(plan)
    }
}
