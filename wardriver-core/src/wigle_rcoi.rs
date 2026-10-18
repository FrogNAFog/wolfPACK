//! WiGLE Android Roaming Consortium (IE 111) formatting.
//!
//! Primary reference: `WifiReceiver.getConcatenatedRcois` / `formatRcoi`.

/// Format one RCOI integer per WiGLE Android `formatRcoi`.
#[must_use]
pub fn format_rcoi(rcoi: u64) -> String {
    if rcoi < 16_777_216 {
        format!("{rcoi:06X}")
    } else {
        format!("{rcoi:010X}")
    }
}

/// Parse Roaming Consortium IE **body** (`ie.getBytes()` in Android) into WiGLE CSV RCOIs.
#[must_use]
pub fn rcois_from_ie_payload(payload: &[u8]) -> String {
    if payload.len() < 2 {
        return String::new();
    }
    let oi12_length = payload[1];
    let oi1_length = (oi12_length & 0x0F) as usize;
    let oi2_length = ((oi12_length >> 4) & 0x0F) as usize;
    let body = &payload[2..];
    if oi1_length.saturating_add(oi2_length) > body.len() {
        return String::new();
    }
    let oi3_length = body.len().saturating_sub(oi1_length + oi2_length);

    let mut out = String::new();
    let mut offset = 0usize;
    for len in [oi1_length, oi2_length, oi3_length] {
        if len == 0 {
            continue;
        }
        let Some(rcoi) = read_oi_be(body, offset, len) else {
            return String::new();
        };
        offset = offset.saturating_add(len);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format_rcoi(rcoi));
    }
    out
}

fn read_oi_be(body: &[u8], offset: usize, size: usize) -> Option<u64> {
    if size == 0 || offset.saturating_add(size) > body.len() {
        return None;
    }
    let mut value = 0u64;
    for &octet in &body[offset..offset + size] {
        value = (value << 8) | u64::from(octet);
    }
    Some(value)
}
