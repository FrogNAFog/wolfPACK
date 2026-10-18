//! Optional boot GPIO exercise for XIAO SD SPI lines (`board-xiao`, Seeed expansion base).
//!
//! Enable with `--features sd-pin-selftest` (also requires `control` + a XIAO-class board).
//! Uses raw ESP-IDF GPIO so SPI pins are not moved before `SpiDriver::new` — each line is reset
//! after its pattern. Chip select GPIO is **25** (Seeed expansion base, CS=D2).
//!
//! Note: do not `use esp_idf_sys::*` here — IDF bindings expose a `log` symbol that clashes with the `log` crate.

use ::log::{info, warn};
use esp_idf_sys::{
    gpio_config, gpio_config_t, gpio_get_level, gpio_int_type_t_GPIO_INTR_DISABLE,
    gpio_mode_t_GPIO_MODE_INPUT, gpio_mode_t_GPIO_MODE_OUTPUT, gpio_num_t,
    gpio_pulldown_t_GPIO_PULLDOWN_DISABLE, gpio_pullup_t_GPIO_PULLUP_DISABLE,
    gpio_pullup_t_GPIO_PULLUP_ENABLE, gpio_reset_pin, gpio_set_level, ESP_OK,
};
use std::thread;
use std::time::Duration;

use super::sd_bus_prep;

const G_SCK: i32 = 8;
const G_MOSI: i32 = 10;
const G_MISO: i32 = 9;

fn gpio_num(n: i32) -> gpio_num_t {
    n as gpio_num_t
}

unsafe fn reset_four(cs: i32) {
    sd_bus_prep::gpio_reset_sd_lines(cs, G_SCK, G_MISO, G_MOSI);
}

unsafe fn gpio_cfg_out(gpio: i32) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_OUTPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_DISABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let rc = gpio_config(&cfg);
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_cfg_out GPIO{gpio} rc={rc}");
    }
}

unsafe fn gpio_cfg_in_floating(gpio: i32) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_INPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_DISABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let rc = gpio_config(&cfg);
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_cfg_in_floating GPIO{gpio} rc={rc}");
    }
}

/// Toggle `driver` while sampling `G_MISO` (floating input). High agreement ⇒ likely short between those nets.
///
/// `idle` sets static levels on the other outputs so the SD card stays deselected (CS high) where possible.
unsafe fn correlate_vs_miso(
    cs: i32,
    driver: i32,
    driver_label: &'static str,
    idle_cs_level: u32,
    idle_sck_level: u32,
    idle_mosi_level: u32,
) {
    reset_four(cs);
    gpio_cfg_out(cs);
    gpio_cfg_out(G_SCK);
    gpio_cfg_out(G_MOSI);
    let _ = gpio_set_level(gpio_num(cs), idle_cs_level);
    let _ = gpio_set_level(gpio_num(G_SCK), idle_sck_level);
    let _ = gpio_set_level(gpio_num(G_MOSI), idle_mosi_level);

    gpio_cfg_out(driver);
    gpio_cfg_in_floating(G_MISO);

    let mut agree = 0u32;
    const N: u32 = 120;
    for i in 0..N {
        let lv = (i % 2) as u32;
        let _ = gpio_set_level(gpio_num(driver), lv);
        thread::sleep(Duration::from_micros(40));
        let r = gpio_get_level(gpio_num(G_MISO));
        let sense_hi = r != 0;
        let want_hi = lv != 0;
        if sense_hi == want_hi {
            agree += 1;
        }
        // Re-apply static idles only on pins that are **not** the toggling driver (avoid undoing the edge).
        if driver != cs {
            let _ = gpio_set_level(gpio_num(cs), idle_cs_level);
        }
        if driver != G_SCK {
            let _ = gpio_set_level(gpio_num(G_SCK), idle_sck_level);
        }
        if driver != G_MOSI {
            let _ = gpio_set_level(gpio_num(G_MOSI), idle_mosi_level);
        }
    }
    let pct = agree * 100 / N;
    info!(
        "sd-pin-selftest: correlate {driver_label}→MISO: {agree}/{N} samples matched toggling driver ({pct}%)"
    );
    if pct >= 85 {
        warn!(
            "sd-pin-selftest: very high correlation — suspect **electrical short** between {driver_label} and MISO (GPIO9 / D9)"
        );
    } else if pct <= 15 {
        info!(
            "sd-pin-selftest: low correlation — no strong evidence of a short between {driver_label} and MISO (breakout pull-ups on SO can skew this; use a scope if unsure)"
        );
    } else {
        info!("sd-pin-selftest: inconclusive correlation ({pct}%) — try shorter leads / scope on D9 vs breakout SO");
    }
    reset_four(cs);
}

unsafe fn gpio_output_pattern(gpio: i32, label: &'static str) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_OUTPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_DISABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let rc = gpio_config(&cfg);
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_config({label} GPIO{gpio}) rc={rc}");
        return;
    }
    for step in 0u32..8 {
        let level = (step % 2) as u32;
        let _ = gpio_set_level(gpio_num(gpio), level);
        info!("sd-pin-selftest: {label} GPIO{gpio} -> {level}");
        thread::sleep(Duration::from_millis(150));
    }
    let _ = gpio_set_level(gpio_num(gpio), 1);
    thread::sleep(Duration::from_millis(40));
    let rc = gpio_reset_pin(gpio_num(gpio));
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_reset_pin({label} GPIO{gpio}) rc={rc}");
    }
}

unsafe fn gpio_input_sample(gpio: i32, label: &'static str) {
    let mut cfg: gpio_config_t = core::mem::zeroed();
    cfg.pin_bit_mask = 1u64 << gpio;
    cfg.mode = gpio_mode_t_GPIO_MODE_INPUT;
    cfg.pull_up_en = gpio_pullup_t_GPIO_PULLUP_ENABLE;
    cfg.pull_down_en = gpio_pulldown_t_GPIO_PULLDOWN_DISABLE;
    cfg.intr_type = gpio_int_type_t_GPIO_INTR_DISABLE;
    let rc = gpio_config(&cfg);
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_config input ({label} GPIO{gpio}) rc={rc}");
        return;
    }
    for i in 0u32..4 {
        let v = gpio_get_level(gpio_num(gpio));
        info!("sd-pin-selftest: {label} GPIO{gpio} read (pull-up) sample {i}: level={v}");
        thread::sleep(Duration::from_millis(80));
    }
    let rc = gpio_reset_pin(gpio_num(gpio));
    if rc != ESP_OK {
        warn!("sd-pin-selftest: gpio_reset_pin({label} GPIO{gpio}) rc={rc}");
    }
}

/// Slow square wave on each SD line so you can probe **D8 / D10 / D9** and the CS pad with a scope or meter.
/// Then **toggle-vs-MISO** checks: if another line is shorted to MISO, levels tend to track together.
///
/// `cs_gpio` / `cs_label`: **GPIO25** + `"CS/D2"` for `board-xiao` (Seeed expansion base).
pub fn run_xiao_sd_lines(cs_gpio: i32, cs_label: &'static str) {
    info!(
        "sd-pin-selftest: begin (order: SCK=8, MOSI=10, {cs_label}=GPIO{cs_gpio}, then MISO=9 input samples)"
    );
    unsafe {
        // Order: SCK and MOSI first; CS before MISO so the card stays deselected during MISO reconfig
        // if the breakout ties CS high when idle (typical SPI idle).
        gpio_output_pattern(8, "SCK/D8");
        gpio_output_pattern(10, "MOSI/D10");
        gpio_output_pattern(cs_gpio, cs_label);
        gpio_input_sample(9, "MISO/D9");

        info!("sd-pin-selftest: --- short sniff: toggle each line vs MISO (floating); CS held high, CLK low, MOSI low when idle ---");
        // CS inactive high; SPI mode 0 idle clock often low; MOSI idle low.
        correlate_vs_miso(cs_gpio, G_MOSI, "MOSI/D10", 1, 0, 0);
        correlate_vs_miso(cs_gpio, G_SCK, "SCK/D8", 1, 0, 0);
        correlate_vs_miso(cs_gpio, cs_gpio, cs_label, 1, 0, 0);

        reset_four(cs_gpio);
    }
    info!("sd-pin-selftest: end — next step is SDSPI mount");
}
