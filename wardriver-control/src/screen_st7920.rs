//! Optional 128x64 screen support (GME12864-11 class, often ST7920-compatible).
//!
//! This is intentionally a stub scaffold: wiring/pin budgeting is locked in the plan,
//! but the actual driver and rendering loop are feature-gated and can be implemented
//! once the specific module voltage/controller details are confirmed.

#[derive(Clone, Copy, Debug)]
pub struct ScreenPins {
    pub sck_gpio: i32,
    pub mosi_gpio: i32,
    pub cs_gpio: i32,
    pub rst_gpio: Option<i32>,
}

/// Placeholder init hook (no-op for now).
pub fn init(_pins: ScreenPins) {
    // Intentionally empty: bring-up will be added when hardware details are confirmed.
}
