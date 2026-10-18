//! Shared escape/format helpers and small form parsers for the web UI.
//!
//! Full HTML pages are static `include_str!` assets in [`super`] — no runtime template rebuild.

use std::sync::{Arc, Mutex};

use crate::config;
use crate::gps;

pub(crate) fn format_bssid(m: &[u8; 6]) -> String {
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

pub(crate) fn parse_alert_sound_mask_form(form: &std::collections::BTreeMap<String, String>) -> u8 {
    let bit = |name: &str| {
        if form.contains_key(name) {
            1u8
        } else {
            0u8
        }
    };
    bit("snd_boot") << 0
        | bit("snd_upload_start") << 1
        | bit("snd_upload_done") << 2
        | bit("snd_gps_fix") << 3
        | bit("snd_detection") << 4
        | bit("snd_surveillance") << 5
        | bit("snd_ssid_watch") << 6
}

pub(crate) fn describe_gps_for_home_zone(gps_fix: &Arc<Mutex<Option<gps::Fix>>>) -> String {
    let Some(f) = gps_fix.lock().ok().and_then(|g| g.clone()) else {
        return "No fix yet.".to_string();
    };
    if config::home_geo_fix_meets_scan_gate(&f) {
        format!(
            "lat {:.7}, lon {:.7} (meets wardriving gate; ok to save as POI)",
            f.lat, f.lon
        )
    } else {
        format!(
            "lat {:.7}, lon {:.7} - fix quality or HDOP does not meet wardriving gate yet",
            f.lat, f.lon
        )
    }
}

pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use core::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            _ => out.push(c),
        }
    }
    out
}
