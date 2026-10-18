//! Host-side stand-in for `esp-idf-sys`.
//!
//! `esp-idf-sys` only builds for the ESP target, but `wardriver-stats` is
//! compiled for x86_64 by `cargo test-host` (golden snapshot tests). On the
//! host the heap/timer FFI symbols resolve to neutral stubs.

#[cfg(not(target_os = "espidf"))]
pub mod esp_idf_sys {
    pub const MALLOC_CAP_INTERNAL: i32 = 1;
    pub const MALLOC_CAP_SPIRAM: i32 = 8;

    /// Monotonic clock in microseconds (host wall clock; test-only).
    pub fn esp_timer_get_time() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as i64)
            .unwrap_or(0)
    }

    pub fn esp_get_free_heap_size() -> u32 {
        0
    }

    pub fn esp_get_minimum_free_heap_size() -> u32 {
        0
    }

    pub fn heap_caps_get_free_size(_caps: u32) -> i64 {
        0
    }

    pub fn heap_caps_get_total_size(_caps: u32) -> i64 {
        0
    }
}
