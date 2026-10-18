//! Ultra-early boot breadcrumbs (no heap, no `log` subscriber).
//!
//! If serial stops after the IDF `coexist:` lines and you **never** see `wd:rom:…`,
//! the CPU has not reached Rust `main()` yet — treat it as an ESP-IDF / sdkconfig /
//! toolchain issue (see `sdkconfig.defaults` comments and Espressif ESP32-C5 guides).
//!
//! `esp_rom_printf` works before `EspLogger::initialize_default()` and helps distinguish
//! “stuck in C startup” vs “stuck in `link_patches` / logging / app code”.

use core::ffi::c_char;

/// Human-readable ESP-IDF reset reason for logs (SD diag, UART).
///
/// Values follow `esp_reset_reason_t` numeric order in ESP-IDF 5.x (stable sequence).
#[inline]
pub fn reset_reason_str() -> &'static str {
    let r = unsafe { esp_idf_sys::esp_reset_reason() } as u32;
    match r {
        0 => "UNKNOWN",
        1 => "POWERON",
        2 => "EXT",
        3 => "SW",
        4 => "PANIC",
        5 => "INT_WDT",
        6 => "TASK_WDT",
        7 => "WDT",
        8 => "DEEPSLEEP",
        9 => "BROWNOUT",
        10 => "SDIO",
        11 => "USB",
        12 => "JTAG",
        13 => "EFUSE",
        14 => "PWR_GLITCH",
        15 => "CPU_LOCKUP",
        _ => "OTHER",
    }
}

/// Raw `esp_reset_reason_t` discriminant (for SD logs when `reset_reason_str` is extended).
#[inline]
pub fn reset_reason_raw() -> u32 {
    unsafe { esp_idf_sys::esp_reset_reason() as u32 }
}

/// Numeric stage marker (keep format string ASCII-only for ROM printf).
#[inline]
pub fn rom_stage(n: i32) {
    // SAFETY: fixed format string; single `%d` argument.
    unsafe {
        esp_idf_sys::esp_rom_printf(
            c"wd:rom:%d\n".as_ptr().cast::<c_char>(),
            n as core::ffi::c_int,
        );
    }
}
