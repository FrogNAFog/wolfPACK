//! Pre-byte-matcher Flock probe IE signature oracle (string compare).
//!
//! Ported from the removed `ieee80211_parse` flock block so host tests can assert parity
//! on constructed probe bodies without a live capture.

use core::fmt::Write as _;

const IE_SSID: u8 = 0;
const IE_VENDOR: u8 = 221;

const FLOCK_PROBE_IE_SIG_PRIMARY_DEFAULT: &str =
    "2,12,127,221:506f9a16030103,45,191,221:0050f208000000";

const FLOCK_PROBE_IE_SIG_PRIMARY_ALT_LINUX: &str =
    "12,221:506f9a16030103,45,191,221:0050f208000000";

const FLOCK_LITEON_IE_SIG_PREFIX: &str = "221:506f9a16030103";

const FLOCK_PHANTOM_SKIP_CAP: u8 = 16;
const FLOCK_TLV_RESYNC_SCAN_MAX: usize = 64;

fn lpr_camera_liteon_vendor_at(ies: &[u8], pos: usize) -> bool {
    pos + 9 <= ies.len()
        && ies[pos] == IE_VENDOR
        && ies[pos + 1] == 7
        && ies[pos + 2] == 0x50
        && ies[pos + 3] == 0x6f
        && ies[pos + 4] == 0x9a
}

fn lpr_camera_phantom_liteon_resync_ahead(ies: &[u8], pos: usize) -> bool {
    let end = (pos + 2)
        .saturating_add(32)
        .min(ies.len().saturating_sub(1));
    (pos + 2..end).any(|j| lpr_camera_liteon_vendor_at(ies, j))
}

fn lpr_camera_is_phantom_overflow(ies: &[u8], id: u8, elen: usize, i: usize) -> bool {
    if i + 2 + elen <= ies.len() {
        return false;
    }
    if elen > 200 {
        return true;
    }
    id == 64 && elen == 128 && lpr_camera_phantom_liteon_resync_ahead(ies, i)
}

fn lpr_camera_scan_tlv_resync(ies: &[u8], start: usize) -> Option<usize> {
    let end = (start + FLOCK_TLV_RESYNC_SCAN_MAX).min(ies.len().saturating_sub(1));
    (start..end).find(|&j| {
        let elen = ies[j + 1] as usize;
        elen <= 200 && j + 2 + elen <= ies.len()
    })
}

fn canonicalize_lpr_camera_probe_ie_sig(sig: &str) -> String {
    if sig.starts_with("2,12,127,") && sig.contains(FLOCK_LITEON_IE_SIG_PREFIX) {
        return sig.to_string();
    }
    if let Some(idx) = sig.find(FLOCK_LITEON_IE_SIG_PREFIX) {
        return format!("2,12,127,{}", &sig[idx..]);
    }
    sig.to_string()
}

fn lpr_camera_probe_ie_sig_from_ies_ex(ies: &[u8]) -> Option<(String, bool)> {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut phantom_skips: u8 = 0;
    while i + 2 <= ies.len() {
        let id = ies[i];
        let elen = ies[i + 1] as usize;
        if i + 2 + elen > ies.len() {
            if phantom_skips < FLOCK_PHANTOM_SKIP_CAP
                && lpr_camera_is_phantom_overflow(ies, id, elen, i)
            {
                phantom_skips += 1;
                i += 2;
                continue;
            }
            if let Some(j) = lpr_camera_scan_tlv_resync(ies, i) {
                if j > i {
                    i = j;
                    continue;
                }
            }
            return None;
        }
        i += 2;
        if id == IE_SSID {
            if elen == 0 {
                while i + 2 <= ies.len() && ies[i] == 0 && ies[i + 1] == 0 {
                    i += 2;
                }
            } else {
                i += elen;
            }
            continue;
        }
        let body = &ies[i..i + elen];
        if id == IE_VENDOR && elen >= 4 {
            let take = elen.min(8);
            let mut hex = String::with_capacity(take * 2);
            for b in &body[..take] {
                let _ = write!(&mut hex, "{:02x}", b);
            }
            parts.push(format!("221:{hex}"));
        } else {
            parts.push(format!("{id}"));
        }
        i += elen;
    }
    let complete = i == ies.len();
    Some((parts.join(","), complete))
}

fn sig_matches_builtin_primary(sig: &str) -> bool {
    sig == FLOCK_PROBE_IE_SIG_PRIMARY_DEFAULT || sig == FLOCK_PROBE_IE_SIG_PRIMARY_ALT_LINUX
}

fn legacy_match_ies(ies: &[u8]) -> bool {
    match lpr_camera_probe_ie_sig_from_ies_ex(ies) {
        Some((s, _complete)) => {
            sig_matches_builtin_primary(&canonicalize_lpr_camera_probe_ie_sig(&s))
        }
        None => false,
    }
}

/// `true` when the pre-migration string signature path accepted `body`.
#[must_use]
pub fn legacy_probe_body_matches_lpr_camera_primary(body: &[u8]) -> bool {
    if body.len() < 2 {
        return false;
    }
    if legacy_match_ies(body) {
        return true;
    }
    if body.len() > 4 {
        return legacy_match_ies(&body[..body.len() - 4]);
    }
    false
}
