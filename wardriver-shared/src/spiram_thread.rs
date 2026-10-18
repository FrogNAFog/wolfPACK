//! Spawn `std::thread` workers with large stacks in PSRAM when available.
//!
//! ESP-IDF pthread defaults to internal DRAM for stack allocation. Call
//! [`esp_pthread_set_cfg`] with `MALLOC_CAP_SPIRAM` before `pthread_create`
//! (requires `CONFIG_SPIRAM_ALLOW_STACK_EXTERNAL_MEMORY`).

use std::thread::{self, JoinHandle};

use esp_idf_sys::{
    esp_pthread_get_default_config, esp_pthread_set_cfg, ESP_ERR_INVALID_ARG, ESP_ERR_NO_MEM,
    ESP_OK, MALLOC_CAP_8BIT, MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM,
};
use log::{info, warn};

use wardriver_stats as stats;

/// ESP-NOW ingest worker stack (control node); [`spawn_control_workers`] may fall back smaller.
pub const INGEST_STACK_BYTES: usize = 128 * 1024;

/// SD I/O worker stack (FATFS + session writers on one pthread; PSRAM when available).
pub const SD_IO_STACK_BYTES: usize = 128 * 1024;

/// PackLink TCP accept/session pthread (lwIP sockets + frame parse).
pub const PACKLINK_TCP_STACK_BYTES: usize = 64 * 1024;

/// ESP-NOW gateway bridge pthread (decrypt, reassemble, bulk export).
pub const GATEWAY_ESPNOW_STACK_BYTES: usize = 64 * 1024;

/// Preferred ingest stacks, largest first (internal-only boards may need a smaller size).
pub const INGEST_STACK_FALLBACKS: &[usize] = &[128 * 1024, 64 * 1024, 32 * 1024, 24 * 1024, 12_288];

fn pthread_set_stack_caps(stack_bytes: usize) -> std::io::Result<&'static str> {
    let use_spiram = stats::free_spiram_heap_bytes() > 0;
    let caps_label = if use_spiram { "spiram" } else { "internal" };
    let mut cfg = unsafe { esp_pthread_get_default_config() };
    cfg.stack_size = stack_bytes;
    cfg.stack_alloc_caps = if use_spiram {
        MALLOC_CAP_8BIT as u32 | MALLOC_CAP_SPIRAM as u32
    } else {
        MALLOC_CAP_8BIT as u32 | MALLOC_CAP_INTERNAL as u32
    };
    let rc = unsafe { esp_pthread_set_cfg(&cfg) };
    match rc {
        ESP_OK => Ok(caps_label),
        ESP_ERR_NO_MEM => Err(std::io::Error::new(
            std::io::ErrorKind::OutOfMemory,
            "esp_pthread_set_cfg: ESP_ERR_NO_MEM",
        )),
        ESP_ERR_INVALID_ARG => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("esp_pthread_set_cfg: invalid stack_bytes={stack_bytes}"),
        )),
        other => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("esp_pthread_set_cfg failed: {other}"),
        )),
    }
}

/// Spawn a named thread with an explicit stack size; log heap headroom at creation.
pub fn spawn_named<F>(name: &str, stack_bytes: usize, f: F) -> std::io::Result<JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    let caps_label = pthread_set_stack_caps(stack_bytes)?;
    if caps_label == "internal" {
        warn!(
            "spiram_thread: SPIRAM unavailable; pthread stack_bytes={stack_bytes} uses internal DRAM"
        );
    }
    info!(
        "spiram_thread: spawn name={name} stack_bytes={stack_bytes} stack_caps={caps_label} internal_free={} spiram_free={}",
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
    );
    thread::Builder::new()
        .name(name.to_owned())
        .stack_size(stack_bytes)
        .spawn(f)
}
