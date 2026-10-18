//! Fixed-point JSON number formatting without `core::fmt` **float** paths.
//!
//! On ESP32, `format!("{f:.1}")` pulls in grisu decimal formatting and a deep stack
//! frame. The control main task builds `/api/stats` here; use integer tenths /
//! hundredths / micro-degrees instead so main stays within its stack guard.
//!
//! The `/api/stats` body builder ([`crate::stats_body`]) consumes the strings these
//! helpers emit as `serde_json::value::RawValue` fragments.

use core::fmt::Write;

/// Round `f32` to tenths (e.g. °C ×10, HDOP ×10, rate_hz ×10).
#[inline]
pub fn f32_to_x10(v: f32) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v * 10.0).round() as i32
}

/// Round `f64` to tenths (e.g. haversine distance in metres).
#[inline]
pub fn f64_to_x10(v: f64) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    (v * 10.0).round() as i32
}

/// Round HDOP to hundredths for two decimal JSON digits.
#[inline]
pub fn hdop_to_x100(hdop: f32) -> u32 {
    if !hdop.is_finite() {
        return 0;
    }
    (hdop * 100.0).round().clamp(0.0, 99_999.0) as u32
}

/// WGS84 degrees → micro-degrees for seven fractional digits without `f64` Display.
#[inline]
pub fn degrees_to_microdeg(deg: f64) -> i64 {
    if !deg.is_finite() {
        return 0;
    }
    (deg * 10_000_000.0).round() as i64
}

/// Append `whole.frac` with exactly one digit after the dot (`x10` fixed-point).
pub fn push_decimal_1_from_x10(out: &mut String, x10: i32) {
    let neg = x10 < 0;
    let abs = x10.unsigned_abs();
    if neg {
        out.push('-');
    }
    let whole = abs / 10;
    let frac = abs % 10;
    let _ = write!(out, "{whole}.{frac}");
}

/// Append two decimal digits from hundredths (`x100` fixed-point).
pub fn push_decimal_2_from_x100(out: &mut String, x100: u32) {
    let whole = x100 / 100;
    let frac = x100 % 100;
    let _ = write!(out, "{whole}.{frac:02}");
}

/// Append degrees with seven fractional digits (`microdeg = round(deg * 1e7)`).
pub fn push_degrees_7_from_microdeg(out: &mut String, microdeg: i64) {
    let neg = microdeg < 0;
    let abs = microdeg.unsigned_abs();
    let whole = abs / 10_000_000;
    let frac = abs % 10_000_000;
    if neg {
        out.push('-');
    }
    let _ = write!(out, "{whole}.{frac:07}");
}

/// OLED / alert token: `HDOP X.Y` (one decimal, ASCII-only).
pub fn hdop_subtitle_token(hdop: f32) -> String {
    let mut s = String::from("HDOP ");
    push_decimal_1_from_x10(&mut s, f32_to_x10(hdop));
    s
}

/// Diag log distance in metres, one decimal, or `"-"` when unknown.
pub fn metres_diag_1dp(d_m: Option<f64>) -> String {
    match d_m {
        None => "-".to_string(),
        Some(d) => {
            let mut s = String::new();
            push_decimal_1_from_x10(&mut s, f64_to_x10(d));
            s
        }
    }
}
