//! wolfPACK firmware — **two mutually exclusive role binaries** (`control` vs `sniffer`).
//!
//! **Control-only (SD + WiGLE):** lives in the `wardriver-control` crate.
//!
//! **Sniffer:** lives in the `wardriver-sniffer` crate; scans and sends ESP-NOW frames to control.

use anyhow::Result;
use esp_idf_svc::log::EspLogger;
use esp_idf_sys as sys;
use log::info;

fn main() -> Result<()> {
    #[cfg(all(feature = "control", feature = "sniffer"))]
    compile_error!("Enable only one of: feature `control`, `sniffer`, or `gateway`.");

    #[cfg(all(feature = "control", feature = "gateway"))]
    compile_error!("Enable only one of: feature `control`, `sniffer`, or `gateway`.");

    #[cfg(all(feature = "sniffer", feature = "gateway"))]
    compile_error!("Enable only one of: feature `control`, `sniffer`, or `gateway`.");

    #[cfg(not(any(feature = "control", feature = "sniffer", feature = "gateway")))]
    compile_error!("Enable exactly one role feature: `control`, `sniffer`, or `gateway`.");

    #[cfg(all(
        feature = "control",
        feature = "board-marauder",
        feature = "board-xiao"
    ))]
    compile_error!(
        "Enable only one board pin profile for `control`: `board-marauder` or `board-xiao`."
    );
    #[cfg(all(
        feature = "control",
        not(any(feature = "board-marauder", feature = "board-xiao")),
    ))]
    compile_error!(
        "When building `control`, enable exactly one board pin profile: `board-marauder` or `board-xiao`."
    );

    #[cfg(all(
        feature = "sniffer",
        any(feature = "board-marauder", feature = "board-xiao"),
    ))]
    compile_error!(
        "Do not combine `sniffer` with board pin features — those pins are control-only."
    );

    #[cfg(all(
        feature = "sd-pin-selftest",
        not(all(feature = "control", feature = "board-xiao")),
    ))]
    compile_error!(
        "Feature `sd-pin-selftest` requires `control` plus `board-xiao` (XIAO SD line GPIO exercise)."
    );

    wardriver_shared::boot_diag::rom_stage(0);
    esp_idf_svc::sys::link_patches();
    wardriver_shared::boot_diag::rom_stage(1);
    EspLogger::initialize_default();
    wardriver_shared::boot_diag::rom_stage(2);
    info!(
        "wolfpack: Rust main (IDF {}.{}.{})",
        sys::ESP_IDF_VERSION_MAJOR,
        sys::ESP_IDF_VERSION_MINOR,
        sys::ESP_IDF_VERSION_PATCH
    );

    wardriver_stats::init_boot_time();
    wardriver_shared::cpu_load::init();
    wardriver_shared::panic_boot::install_panic_reboot();
    wardriver_shared::pm_policy::init();
    wardriver_shared::soc_temp::init();
    info!(
        "wolfpack: reset_reason={} (rst_n={})",
        wardriver_shared::boot_diag::reset_reason_str(),
        wardriver_shared::boot_diag::reset_reason_raw()
    );

    #[cfg(feature = "control")]
    {
        wardriver_control::run()
    }

    #[cfg(feature = "sniffer")]
    {
        wardriver_sniffer::run()
    }

    #[cfg(feature = "gateway")]
    {
        wardriver_gateway::run()
    }
}
