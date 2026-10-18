//! On panic, log best-effort then `esp_restart()` so a crashed node recovers.
//!
//! `std` targets already provide `panic_impl`, so we use [`std::panic::set_hook`] instead of
//! `#[panic_handler]` (which only applies to `#![no_std]`).

pub fn install_panic_reboot() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("wolfpack FATAL: {info}");
        unsafe {
            esp_idf_sys::esp_restart();
        }
    }));
}
