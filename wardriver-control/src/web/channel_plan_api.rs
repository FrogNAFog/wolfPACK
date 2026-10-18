//! Channel plan HTTP API helpers (`/api/channel-plan`, preview, form parse).

use std::collections::BTreeMap;

use crate::channel_plan::{self, BandAssignmentMode, ChannelPlan, PerSnifferChannel};
use wardriver_stats as stats;

fn parse_csv_u8_field(s: &str) -> Vec<u8> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<u8>().ok())
        .collect()
}

fn parse_csv_u32_field(s: &str) -> Vec<u32> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse::<u32>().ok())
        .collect()
}

fn flag_field(form: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    match form.get(key) {
        Some(v) => !matches!(v.as_str(), "0" | "false" | "off" | ""),
        None => default,
    }
}

/// Build a [`ChannelPlan`] from a form-urlencoded POST body. Keys mirror
/// [`ChannelPlan::encode_kv`] field names. Missing keys fall back to the current saved plan
/// so the UI can submit partial updates.
pub(crate) fn parse_channel_plan_form(form: &BTreeMap<String, String>) -> ChannelPlan {
    let mut plan = channel_plan::current_plan();
    plan.include_2_4_ghz = flag_field(form, "include_2_4", plan.include_2_4_ghz);
    plan.include_5_ghz = flag_field(form, "include_5", plan.include_5_ghz);
    plan.include_dfs = flag_field(form, "include_dfs", plan.include_dfs);
    if let Some(v) = form.get("dwell_ms").and_then(|s| s.parse::<u16>().ok()) {
        plan.dwell_ms = v;
    }
    if let Some(v) = form.get("channels_2_4") {
        plan.channels_2_4 = parse_csv_u8_field(v);
    }
    if let Some(v) = form.get("channels_5") {
        plan.channels_5 = parse_csv_u8_field(v);
    }
    if let Some(v) = form.get("channels_5_dfs") {
        plan.channels_5_dfs = parse_csv_u8_field(v);
    }
    if let Some(v) = form.get("band_mode") {
        plan.band_mode = match v.as_str() {
            "manual" => BandAssignmentMode::Manual,
            _ => BandAssignmentMode::AutoBalance,
        };
    }
    if let Some(v) = form.get("sniffers_2_4") {
        plan.sniffers_2_4 = parse_csv_u32_field(v);
    }
    if let Some(v) = form.get("sniffers_5") {
        plan.sniffers_5 = parse_csv_u32_field(v);
    }
    // `pins=sid:ch,sid:ch,...` (channel 0 means unpinned).
    if let Some(v) = form.get("pins") {
        plan.per_sniffer.clear();
        for tok in v.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if let Some((id_s, ch_s)) = tok.split_once(':') {
                if let (Ok(id), Ok(ch)) = (id_s.trim().parse::<u32>(), ch_s.trim().parse::<u8>()) {
                    let pinned_channel = if ch == 0 { None } else { Some(ch) };
                    plan.per_sniffer.push(PerSnifferChannel {
                        sniffer_id: id,
                        pinned_channel,
                    });
                }
            }
        }
    }
    plan.normalize_channels();
    plan.clamp_dwell();
    plan.sniffers_2_4.sort_unstable();
    plan.sniffers_2_4.dedup();
    plan.sniffers_5.sort_unstable();
    plan.sniffers_5.dedup();
    // Ephemeral lists: auto mode rebuilds assignments at runtime.
    if plan.band_mode == BandAssignmentMode::AutoBalance {
        plan.sniffers_2_4.clear();
        plan.sniffers_5.clear();
        plan.per_sniffer.clear();
    }
    plan
}

pub(crate) fn fmt_csv_u8(v: &[u8]) -> String {
    let mut s = String::with_capacity(v.len().saturating_mul(4));
    for (i, c) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        use core::fmt::Write as _;
        let _ = write!(s, "{c}");
    }
    s
}

pub(crate) fn fmt_csv_u32(v: &[u32]) -> String {
    let mut s = String::with_capacity(v.len().saturating_mul(6));
    for (i, c) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        use core::fmt::Write as _;
        let _ = write!(s, "{c}");
    }
    s
}

/// Append a JSON array of decimal u8 channel numbers (no quotes).
pub(crate) fn push_json_u8_array(out: &mut String, slice: &[u8], len: usize) {
    out.push('[');
    for (j, &ch) in slice.iter().take(len).enumerate() {
        if j > 0 {
            out.push(',');
        }
        use core::fmt::Write as _;
        let _ = write!(out, "{ch}");
    }
    out.push(']');
}

/// Join pre-rendered JSON array elements into `[e0,e1,...]` (comma, no spaces).
/// Byte-identical to a `String::from("[")` + `push(',')` + `push_str(elem)` loop.
fn join_json_array(elements: &[String]) -> String {
    let mut s = String::from("[");
    for (i, e) in elements.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(e);
    }
    s.push(']');
    s
}

pub(crate) fn channel_plan_response_json(preview: bool, candidate: Option<&ChannelPlan>) -> String {
    let mut plan = candidate
        .cloned()
        .unwrap_or_else(channel_plan::current_plan);

    let mut active_ids: Vec<u32> = Vec::new();
    stats::fill_wifi_role_sniffer_ids(&mut active_ids);

    plan.prune_unknown_sniffers(&active_ids);

    let effective = channel_plan::effective_from_plan(&plan, &active_ids);

    let (unassigned_json, unassigned_count) = if plan.band_mode == BandAssignmentMode::AutoBalance {
        let elements = effective
            .unassigned
            .iter()
            .map(|&ch| ch.to_string())
            .collect::<Vec<String>>();
        let arr = join_json_array(&elements);
        (arr, effective.unassigned.len())
    } else {
        (String::from("[]"), 0)
    };

    let pins_json = join_json_array(
        &plan
            .per_sniffer
            .iter()
            .map(|p| {
                format!(
                    "{{\"sniffer_id\":{},\"pinned_channel\":{}}}",
                    p.sniffer_id,
                    p.pinned_channel.unwrap_or(0)
                )
            })
            .collect::<Vec<String>>(),
    );

    let sched_json = join_json_array(
        &effective
            .schedules
            .iter()
            .map(|(id, sched)| {
                let band_wire = sched.band.map(|b| b.as_wire()).unwrap_or(0);
                let mut bucket = String::with_capacity(sched.chan_count as usize * 4 + 2);
                push_json_u8_array(&mut bucket, &sched.channels, sched.chan_count as usize);
                format!(
                    "{{\"sniffer_id\":{},\"band\":{},\"chan_count\":{},\"dwell_ms\":{},\"pinned\":{},\"channels\":{}}}",
                    id,
                    band_wire,
                    sched.chan_count,
                    sched.dwell_ms,
                    sched.pinned as u8,
                    bucket,
                )
            })
            .collect::<Vec<String>>(),
    );

    let defaults_24 = fmt_csv_u8(&channel_plan::default_channels_2_4());
    let defaults_5 = fmt_csv_u8(&channel_plan::default_channels_5());
    let defaults_5_dfs = fmt_csv_u8(&channel_plan::default_channels_5_dfs());
    let band_mode_json = match plan.band_mode {
        BandAssignmentMode::Manual => "manual",
        BandAssignmentMode::AutoBalance => "auto",
    };
    // Ephemeral in auto mode; assignments rebuild from the active fleet at runtime.
    let (sn24, sn5) = if plan.band_mode == BandAssignmentMode::AutoBalance {
        (String::new(), String::new())
    } else {
        (
            fmt_csv_u32(&plan.sniffers_2_4),
            fmt_csv_u32(&plan.sniffers_5),
        )
    };

    format!(
        "{{\
\"preview\":{preview},\
\"revision\":{rev},\
\"band_mode\":\"{band_mode}\",\
\"include_2_4\":{i24},\
\"include_5\":{i5},\
\"include_dfs\":{idfs},\
\"dwell_ms\":{dwell},\
\"channels_2_4\":[{ch24}],\
\"channels_5\":[{ch5}],\
\"channels_5_dfs\":[{ch5d}],\
\"sniffers_2_4\":[{sn24}],\
\"sniffers_5\":[{sn5}],\
\"pins\":{pins},\
\"defaults\":{{\"channels_2_4\":[{d24}],\"channels_5\":[{d5}],\"channels_5_dfs\":[{d5d}],\"dwell_ms\":{ddwell}}},\
\"effective\":{{\"hop_cycle_ms\":{hop},\"unassigned_channels\":{unassigned},\"unassigned_count\":{unassigned_count},\"schedules\":{sched}}}\
}}",
        preview = preview,
        rev = channel_plan::revision(),
        band_mode = band_mode_json,
        i24 = plan.include_2_4_ghz,
        i5 = plan.include_5_ghz,
        idfs = plan.include_dfs,
        dwell = plan.dwell_ms,
        ch24 = fmt_csv_u8(&plan.channels_2_4),
        ch5 = fmt_csv_u8(&plan.channels_5),
        ch5d = fmt_csv_u8(&plan.channels_5_dfs),
        sn24 = sn24,
        sn5 = sn5,
        pins = pins_json,
        d24 = defaults_24,
        d5 = defaults_5,
        d5d = defaults_5_dfs,
        ddwell = channel_plan::DEFAULT_DWELL_MS,
        hop = effective.hop_cycle_ms,
        unassigned = unassigned_json,
        unassigned_count = unassigned_count,
        sched = sched_json,
    )
}
