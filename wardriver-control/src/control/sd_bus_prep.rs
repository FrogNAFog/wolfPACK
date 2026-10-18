//! SDIO/SPI bus preconditioning before SDSPI mount (control boot).
//!
//! Uses raw ESP-IDF GPIO so lines can be idled before `SpiDriver::new`. After a soft reset the
//! microSD stays powered; idle clocks with CS high help return the card to a known SPI state.
//!
//! Note: do not `use esp_idf_sys::*` here — IDF bindings expose a `log` symbol that clashes with the `log` crate.

use esp_idf_hal::delay::FreeRtos;
use esp_idf_sys::{
    gpio_config, gpio_config_t, gpio_int_type_t_GPIO_INTR_DISABLE, gpio_mode_t_GPIO_MODE_INPUT,
    gpio_mode_t_GPIO_MODE_OUTPUT, gpio_num_t, gpio_pulldown_t_GPIO_PULLDOWN_DISABLE,
    gpio_pullup_t_GPIO_PULLUP_DISABLE, gpio_reset_pin, gpio_set_level,
};
use wardriver_board::BoardProfile;
use wardriver_shared::boot_diag;

/// Extra settle after prep when the MCU rebooted without power-cycling the SD card.
const SOFT_RESET_SETTLE_MS: u32 = 100;

/// SD spec: at least 74 idle clocks with CS high before CMD0 when the card was not power-cycled.
const IDLE_CLOCK_CYCLES: u32 = 80;

fn gpio_num(n: i32) -> gpio_num_t {
    n as gpio_num_t
}

/// Reset all four SD SPI pads to the default GPIO state.
///
/// # Safety
///
/// Caller must not have an active SPI driver on these pins.
pub(crate) unsafe fn gpio_reset_sd_lines(cs: i32, sck: i32, miso: i32, mosi: i32) {
    for g in [cs, sck, miso, mosi] {
        let _ = gpio_reset_pin(gpio_num(g));
    }
}

unsafe fn gpio_cfg_out(gpio: i32) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_OUTPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_DISABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let _ = gpio_config(&cfg);
}

unsafe fn gpio_cfg_in_floating(gpio: i32) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_INPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_DISABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let _ = gpio_config(&cfg);
}

unsafe fn send_idle_spi_clocks(cs: i32, sck: i32, mosi: i32) {
    gpio_cfg_out(cs);
    gpio_cfg_out(sck);
    gpio_cfg_out(mosi);
    let _ = gpio_set_level(gpio_num(cs), 1);
    let _ = gpio_set_level(gpio_num(mosi), 0);
    let _ = gpio_set_level(gpio_num(sck), 0);
    for _ in 0..IDLE_CLOCK_CYCLES {
        let _ = gpio_set_level(gpio_num(sck), 1);
        let _ = gpio_set_level(gpio_num(sck), 0);
    }
}

/// Reset pads, drive SPI mode-0 idle, send idle clocks, reset pads again for `SpiDriver::new`.
pub(crate) fn prep_spi_bus_idle(bp: &BoardProfile) {
    let cs = i32::from(bp.sd_cs);
    let sck = i32::from(bp.sd_sck);
    let miso = i32::from(bp.sd_miso);
    let mosi = i32::from(bp.sd_mosi);
    unsafe {
        gpio_reset_sd_lines(cs, sck, miso, mosi);
        send_idle_spi_clocks(cs, sck, mosi);
        gpio_cfg_in_floating(miso);
        gpio_reset_sd_lines(cs, sck, miso, mosi);
    }
}

/// Milliseconds to wait after bus prep when the card likely retained state across reset.
#[must_use]
pub(crate) fn soft_reset_settle_ms() -> u32 {
    match boot_diag::reset_reason_raw() {
        1 => 0, // POWERON — card was power-cycled with the MCU
        _ => SOFT_RESET_SETTLE_MS,
    }
}

/// Run [`prep_spi_bus_idle`] then optional soft-reset settle delay.
pub(crate) fn prep_and_settle(bp: &BoardProfile) {
    prep_spi_bus_idle(bp);
    let ms = soft_reset_settle_ms();
    if ms > 0 {
        FreeRtos::delay_ms(ms);
    }
}
