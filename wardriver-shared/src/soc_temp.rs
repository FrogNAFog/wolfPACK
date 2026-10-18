//! ESP32-C5 on-die temperature via `esp_driver_tsens` (dashboard / stats only).

use std::sync::Mutex;
use std::sync::OnceLock;

use esp_idf_sys::{
    soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT,
    temperature_sensor_config_t, temperature_sensor_config_t__bindgen_ty_1,
    temperature_sensor_enable, temperature_sensor_get_celsius, temperature_sensor_handle_t,
    temperature_sensor_install, ESP_OK,
};

static HANDLE: OnceLock<Mutex<Option<TempHandle>>> = OnceLock::new();

struct TempHandle {
    raw: temperature_sensor_handle_t,
}

unsafe impl Send for TempHandle {}

/// One-time init; safe to call multiple times (no-op after success).
pub fn init() {
    let slot = HANDLE.get_or_init(|| Mutex::new(None));
    let mut g = match slot.lock() {
        Ok(x) => x,
        Err(_) => return,
    };
    if g.is_some() {
        return;
    }

    let cfg = temperature_sensor_config_t {
        range_min: -10,
        range_max: 80,
        clk_src: soc_periph_temperature_sensor_clk_src_t_TEMPERATURE_SENSOR_CLK_SRC_DEFAULT,
        flags: temperature_sensor_config_t__bindgen_ty_1 { allow_pd: 0 },
    };
    let mut h: temperature_sensor_handle_t = core::ptr::null_mut();
    let rc = unsafe { temperature_sensor_install(&cfg, &mut h) };
    if rc != ESP_OK || h.is_null() {
        log::warn!("soc_temp: temperature_sensor_install rc={rc}");
        return;
    }
    let rc = unsafe { temperature_sensor_enable(h) };
    if rc != ESP_OK {
        log::warn!("soc_temp: temperature_sensor_enable rc={rc}");
        unsafe {
            let _ = esp_idf_sys::temperature_sensor_uninstall(h);
        }
        return;
    }
    *g = Some(TempHandle { raw: h });
    log::info!("soc_temp: on-chip temperature sensor enabled");
}

/// Latest reading in °C, or `None` if unavailable / driver error.
pub fn read_celsius() -> Option<f32> {
    let slot = HANDLE.get_or_init(|| Mutex::new(None));
    let g = slot.lock().ok()?;
    let h = g.as_ref()?;
    let mut out: f32 = 0.0;
    let rc = unsafe { temperature_sensor_get_celsius(h.raw, &mut out) };
    if rc != ESP_OK {
        return None;
    }
    Some(out)
}
