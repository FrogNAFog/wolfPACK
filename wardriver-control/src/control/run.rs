//! Control-node entry point (GPS + SD + optional WiGLE upload) for ESP-NOW v1.
//!
//! Responsibilities:
//! - Start LED, SD mount, web UI/provisioning, GPS/time sync, optional WiGLE upload
//! - Initialize ESP-NOW and accept scan reports from sniffers
//! - Geo-tag + timestamp observations and write WiGLE CSV to SD (**only this role** mounts
//!   the card and writes WiGLE-format `*.csv`; sniffer firmware does not compile `storage`.)
//! - Send START/STOP to sniffers from fleet/recon hold only; WiGLE logging additionally requires
//!   GPS HDOP, wall clock, and optional home-zone privacy (scan + detection alerts can run without logging)
//! - Learn home AP BSSID for WiGLE privacy, merge manual exclusions from SD (`exclusions/wigle/…`,
//!   with one-time NVS→SD migration), broadcast a BSSID denylist over ESP-NOW, and filter again
//!   when merging scan fragments to CSV
//!
//! `run_control` is a linear sequence of named boot phases — [`init_hardware`], [`init_networking`],
//! [`init_fleet_downlink`], [`spawn_workers`] — each returning the handles it creates so the next
//! phase (and the final [`ControlLoopDeps`] assembly) can reach them. The boot bring-up order and
//! every `info!`/`warn!` line are preserved exactly; only the grouping into phase fns changed.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, RwLock};

use super::boot_run::{self, ExclusionState};
use super::boot_setup;
use super::control_workers::{self, IngestWorkerDeps};
use super::main_loop::{
    control_loop_tick, ControlLoopDeps, ControlLoopState, CoreDeps, ExclusionDeps, FleetDeps,
    GpsDeps,
};
use super::nvs_defer_queue::{NvsDeferQueue, NvsDeferRequest};
use super::CONTROL_AP_CHANNEL;
use anyhow::Result;
use esp_idf_hal::modem::Modem;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;
use log::{info, warn};

use crate::config;
use crate::gps;
use crate::storage;
use crate::time_sync;
use crate::web;
use crate::web::snapshots::WebSnapshotState;
use wardriver_shared::espnow;
use wardriver_shared::memory_policy;
use wardriver_shared::spiram_queue;
use wardriver_stats as stats;

pub fn run_control() -> Result<()> {
    let hw = init_hardware()?;
    let net = init_networking(hw)?;
    let fleet = init_fleet_downlink(net)?;
    let mut boot = spawn_workers(fleet)?;
    let mut state = boot.state;
    let mut deps = ControlLoopDeps {
        fleet: FleetDeps {
            sniffers: boot.sniffers,
            cmd_seq: boot.cmd_seq,
            control_boot_id: boot.control_boot_id,
            own_management_ap_bssid: boot.own_management_ap_bssid,
            peer_if: ESPNOW_PEER_IF,
            scanning_allowed: boot.scanning_allowed,
        },
        exclusion: ExclusionDeps {
            wigle_home: Arc::clone(&boot.exclusion.wigle_home_bssid),
            wigle_manual: Arc::clone(&boot.exclusion.wigle_exclude_manual),
            wigle_ssids_manual: Arc::clone(&boot.exclusion.wigle_exclude_ssids_manual),
            detection_ignore: Arc::clone(&boot.exclusion.detection_exclude_manual),
            ssid_watchlist: Arc::clone(&boot.exclusion.ssid_watchlist),
            denylist_dirty: Arc::clone(&boot.exclusion.denylist_dirty),
            watchlist_dirty: Arc::clone(&boot.exclusion.watchlist_dirty),
            detection_masks_dirty: Arc::clone(&boot.exclusion.detection_masks_dirty),
            home_geo: Arc::clone(&boot.exclusion.home_geo),
        },
        gps: GpsDeps {
            gps_fix: Arc::clone(&boot.gps_fix),
            gps_supervisor: Arc::clone(&boot.gps_supervisor),
        },
        core: CoreDeps {
            wifi: &mut boot.wifi,
            nvs: &boot.nvs,
            alert_hub: Arc::clone(&boot.alert_hub),
            sd_io: boot.sd_io,
            nvs_defer_rx: boot.nvs_defer_rx,
            web_snapshots: boot.web_snapshots,
        },
    };

    loop {
        control_loop_tick(&mut state, &mut deps);
    }
}

// ---------------------------------------------------------------------------
// Boot phases. Each runs its slice of the original inline boot sequence verbatim
// (same order, same log lines) and returns the handles it creates. The return
// structs carry the earlier phases' handles forward so later phases and the final
// `ControlLoopDeps` assembly can reach them.
// ---------------------------------------------------------------------------

/// Hardware bring-up handles: peripherals, NVS + config, exclusion, alert hub, SD,
/// GPS (UART + supervisor + history), and the modem/sys-loop the WiFi phase needs.
struct BootHardware {
    nvs: EspNvsPartition<NvsDefault>,
    nvs_arc: Arc<EspNvsPartition<NvsDefault>>,
    cfg: Arc<config::Config>,
    exclusion: ExclusionState,
    alert_hub: Arc<crate::alert_feedback::AlertHub>,
    sd_io: Arc<storage::SdIoHandle>,
    gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    gps_supervisor: Arc<gps::GpsSupervisor>,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    gps_has_fix: bool,
    modem: Modem<'static>,
    sys_loop: EspSystemEventLoop,
}

/// WiFi / SoftAP / memory-policy handles (carries the hardware handles forward).
struct BootNetworking {
    nvs: EspNvsPartition<NvsDefault>,
    nvs_arc: Arc<EspNvsPartition<NvsDefault>>,
    cfg: Arc<config::Config>,
    exclusion: ExclusionState,
    alert_hub: Arc<crate::alert_feedback::AlertHub>,
    sd_io: Arc<storage::SdIoHandle>,
    gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    gps_supervisor: Arc<gps::GpsSupervisor>,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    gps_has_fix: bool,
    wifi: BlockingWifi<EspWifi<'static>>,
    own_management_ap_bssid: Option<[u8; 6]>,
    mem_policy: memory_policy::RuntimeMemoryPolicy,
}

/// ESP-NOW / web / fleet-downlink handles (carries hardware + networking handles forward).
struct BootFleet {
    nvs: EspNvsPartition<NvsDefault>,
    nvs_arc: Arc<EspNvsPartition<NvsDefault>>,
    cfg: Arc<config::Config>,
    exclusion: ExclusionState,
    alert_hub: Arc<crate::alert_feedback::AlertHub>,
    sd_io: Arc<storage::SdIoHandle>,
    gps_hist: Arc<RwLock<VecDeque<(i64, gps::Fix)>>>,
    gps_supervisor: Arc<gps::GpsSupervisor>,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    gps_has_fix: bool,
    wifi: BlockingWifi<EspWifi<'static>>,
    own_management_ap_bssid: Option<[u8; 6]>,
    mem_policy: memory_policy::RuntimeMemoryPolicy,
    control_boot_id: u32,
    sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    cmd_seq: Arc<AtomicU32>,
    web_snapshots: Arc<RwLock<WebSnapshotState>>,
    web: EspHttpServer<'static>,
    nvs_defer: NvsDeferQueue,
    nvs_defer_rx: Receiver<NvsDeferRequest>,
    rx_rx: spiram_queue::Receiver<espnow::RxFrame>,
    scanning_allowed: Arc<AtomicBool>,
}

/// Worker handles carried to the final `ControlLoopDeps` assembly (only the fields
/// `run_control` reads are kept).
struct BootWorkers {
    nvs: EspNvsPartition<NvsDefault>,
    exclusion: ExclusionState,
    alert_hub: Arc<crate::alert_feedback::AlertHub>,
    sd_io: Arc<storage::SdIoHandle>,
    gps_supervisor: Arc<gps::GpsSupervisor>,
    gps_fix: Arc<Mutex<Option<gps::Fix>>>,
    wifi: BlockingWifi<EspWifi<'static>>,
    own_management_ap_bssid: Option<[u8; 6]>,
    control_boot_id: u32,
    sniffers: Arc<Mutex<HashSet<[u8; 6]>>>,
    cmd_seq: Arc<AtomicU32>,
    web_snapshots: Arc<RwLock<WebSnapshotState>>,
    _web: EspHttpServer<'static>,
    _nvs_defer: NvsDeferQueue,
    nvs_defer_rx: Receiver<NvsDeferRequest>,
    scanning_allowed: Arc<AtomicBool>,
    state: ControlLoopState,
}

/// Phase 1a — SD card bring-up: optional pin self-test, the board SPI pin log, the FAT
/// mount, the sd-io worker spawn + registration, the post-mount / post-fail hooks, and the
/// control-main-task watchdog registration. Boot-log order is preserved exactly.
fn init_sd_boot(
    spi_periph: esp_idf_hal::spi::SPI2<'static>,
    nvs: &EspNvsPartition<NvsDefault>,
    exclusion: &ExclusionState,
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
) -> (Result<()>, Arc<storage::SdIoHandle>) {
    #[cfg(all(feature = "board-xiao", feature = "sd-pin-selftest"))]
    {
        let bp = boot_setup::board_profile();
        super::sd_pin_selftest::run_xiao_sd_lines(i32::from(bp.sd_cs), bp.sd_cs_label);
    }

    {
        let bp = boot_setup::board_profile();
        info!(
            "wolfpack/sd: board={} SPI pins SCK=GPIO{} MOSI=GPIO{} MISO=GPIO{} CS=GPIO{} ({}) -> FAT mount point {}",
            bp.name,
            bp.sd_sck,
            bp.sd_mosi,
            bp.sd_miso,
            bp.sd_cs,
            bp.sd_cs_label,
            storage::MOUNT_POINT
        );
    }
    info!("wolfpack/boot: mounting SD (FATFS + SDSPI)");
    wardriver_shared::task_wdt::register_current_task_for_watchdog();
    use esp_idf_hal::gpio::AnyIOPin;
    let board = boot_setup::board_profile();
    // SAFETY: SD pins are exclusive to this mount call; the bootloader left them unconfigured.
    let sd_sck = unsafe { AnyIOPin::steal(board.sd_sck) };
    let sd_mosi = unsafe { AnyIOPin::steal(board.sd_mosi) };
    let sd_miso = unsafe { AnyIOPin::steal(board.sd_miso) };
    let sd_cs = unsafe { AnyIOPin::steal(board.sd_cs) };
    let sd_mount_res = boot_setup::mount_sd_for_board(spi_periph, sd_sck, sd_mosi, sd_miso, sd_cs);

    if sd_mount_res.is_ok() {
        stats::set_sd_mounted(true);
        storage::prepare_sd_boot_before_worker_spawn();
    }
    let sd_io = Arc::new(storage::spawn_sd_io_worker());
    storage::register_sd_io_handle(Arc::clone(&sd_io));

    match &sd_mount_res {
        Ok(()) => {
            info!(
                "wolfpack/sd: mount OK — WiGLE CSV directory {}",
                storage::Paths::new().pending.display()
            );
            boot_run::on_sd_mount_ok(nvs, exclusion, sd_io.as_ref());
        }
        Err(e) => boot_run::on_sd_mount_fail(nvs, alert_hub, exclusion, e),
    }

    info!("wolfpack/boot: task watchdog registered (control main task)");
    (sd_mount_res, sd_io)
}

/// Phase 1b — OLED: try the SSD1306 display for the active board and attach it to the
/// alert hub when present.
fn init_oled_boot(
    i2c0: esp_idf_hal::i2c::I2C0<'static>,
    alert_hub: &Arc<crate::alert_feedback::AlertHub>,
) {
    if let Some(display) = crate::display_ssd1306::try_init_for_board(i2c0) {
        alert_hub.attach_display(display);
    }
}

/// Phase 1c — GPS: open the GPS UART, configure the L76K (board-xiao), and start the
/// GPS supervisor task (with the position-history ring). Boot-log order is preserved.
fn init_gps_boot(
    uart1: esp_idf_hal::uart::UART1<'static>,
) -> Result<(
    Arc<RwLock<VecDeque<(i64, crate::gps::Fix)>>>,
    Arc<crate::gps::GpsSupervisor>,
    Arc<Mutex<Option<crate::gps::Fix>>>,
)> {
    use esp_idf_hal::gpio::AnyIOPin;
    let board = boot_setup::board_profile();
    let gps_uart_cfg = esp_idf_svc::hal::uart::config::Config::default()
        .baudrate(esp_idf_hal::units::Hertz(boot_setup::GPS_UART_BAUD));
    let gps_tx = unsafe { AnyIOPin::steal(board.gps_uart_tx) };
    let gps_rx = unsafe { AnyIOPin::steal(board.gps_uart_rx) };
    let mut gps_uart = boot_setup::open_gps_uart(uart1, gps_tx, gps_rx, &gps_uart_cfg)?;

    #[cfg(feature = "board-xiao")]
    if let Err(e) = crate::gps::l76k_configure(&mut gps_uart) {
        warn!("gps: L76K: configure failed (continuing): {e:#}");
    }

    let gps_hist = Arc::new(RwLock::new(VecDeque::with_capacity(320)));
    let gps_supervisor = crate::gps::GpsSupervisor::start(gps_uart, Some(Arc::clone(&gps_hist)))?;
    let gps_fix = Arc::clone(&gps_supervisor.fix);
    info!("wolfpack/boot: GPS task started");
    Ok((gps_hist, gps_supervisor, gps_fix))
}

/// Phase 1 — physical + peripheral bring-up: peripherals, NVS + config, alert hub,
/// SD mount + sd-io worker, OLED, GPS UART + supervisor, boot button, post-boot alerts.
fn init_hardware() -> Result<BootHardware> {
    time_sync::set_tz_utc();

    info!("wolfPACK control v{}", env!("CARGO_PKG_VERSION"));

    // NOTE: Don't bring up NimBLE on control. It isn't used here and can push heap low
    // enough to break SoftAP auth handshakes and ESP-NOW TX (ESP_ERR_ESPNOW_NO_MEM).

    let peripherals = Peripherals::take()?;
    info!("wolfpack/boot: peripherals taken");
    let sys_loop = EspSystemEventLoop::take()?;
    info!("wolfpack/boot: system event loop");

    let pins = peripherals.pins;
    let modem = peripherals.modem;
    let uart1 = peripherals.uart1;
    let spi_periph = peripherals.spi2;

    let nvs = EspNvsPartition::<NvsDefault>::take()?;
    info!("wolfpack/boot: NVS partition");
    let nvs_arc = Arc::new(nvs.clone());

    let cfg = Arc::new(config::Config::load(&nvs)?);
    stats::set_nvs_config_flags(
        cfg.wigle_configured(),
        cfg.wdgwars_configured(),
        cfg.provisioning_complete(),
    );
    stats::set_management_ap_ssid(cfg.resolved_ap_ssid());
    info!(
        "wolfpack/boot: config loaded (provisioning_complete={})",
        cfg.provisioning_complete()
    );
    crate::fleet_roster::init(&nvs);
    let roster_adopted = crate::fleet_roster::with_roster(|r| r.list().len()).unwrap_or(0);
    info!("wolfpack/boot: fleet roster loaded ({roster_adopted} adopted)");
    stats::set_alert_sound_mask(config::alert_sound_mask_load(&nvs));
    stats::set_alert_buzzer_volume_percent(config::alert_buzzer_volume_load(&nvs));

    let i2c0 = peripherals.i2c0;

    let exclusion = ExclusionState::new(&nvs);

    let alert_hub: Arc<crate::alert_feedback::AlertHub> = {
        #[cfg(feature = "board-xiao")]
        {
            let ledc = peripherals.ledc;
            let buzzer = crate::buzzer::Buzzer::new(ledc.channel0, ledc.timer0, pins.gpio7)?;
            crate::alert_feedback::AlertHub::spawn(buzzer)?
        }
        #[cfg(not(feature = "board-xiao"))]
        {
            crate::alert_feedback::AlertHub::spawn()?
        }
    };

    let (_, sd_io) = init_sd_boot(spi_periph, &nvs, &exclusion, &alert_hub);

    init_oled_boot(i2c0, &alert_hub);

    let (gps_hist, gps_supervisor, gps_fix) = init_gps_boot(uart1)?;

    let _boot_button = wardriver_shared::boot_button::BootButtonHandle::spawn(pins.gpio28)
        .map_err(|e| {
            warn!("boot_button: {e:#}");
            e
        })
        .ok();

    let gps_has_fix = boot_run::gps_has_usable_fix(&gps_fix);
    boot_run::trigger_post_boot_alerts(&alert_hub, gps_has_fix);

    Ok(BootHardware {
        nvs,
        nvs_arc,
        cfg,
        exclusion,
        alert_hub,
        sd_io,
        gps_hist,
        gps_supervisor,
        gps_fix,
        gps_has_fix,
        modem,
        sys_loop,
    })
}

/// Phase 2 — WiFi driver + management SoftAP + boot upload window + memory policy.
fn init_networking(hw: BootHardware) -> Result<BootNetworking> {
    info!("wolfpack/boot: EspWifi::new + BlockingWifi::wrap");
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(hw.modem, hw.sys_loop.clone(), Some(hw.nvs.clone()))?,
        hw.sys_loop,
    )?;
    info!("wolfpack/boot: WiFi driver wrapped");

    // Country code only until boot upload finishes; SoftAP deferred when upload may run STA-only.
    boot_setup::configure_wifi_country_us();

    let provisioning_complete = hw.cfg.as_ref().provisioning_complete();
    if !provisioning_complete {
        boot_run::ensure_management_softap(&mut wifi, hw.cfg.as_ref())?;
    }

    let softap_from_upload_window = boot_run::boot_sta_upload_window(
        &mut wifi,
        hw.cfg.as_ref(),
        &hw.nvs,
        &hw.alert_hub,
        &hw.exclusion,
        Arc::clone(&hw.sd_io),
    );
    if provisioning_complete && !softap_from_upload_window {
        boot_run::ensure_management_softap(&mut wifi, hw.cfg.as_ref())?;
    }

    let own_management_ap_bssid = match boot_setup::read_softap_mac() {
        Ok(m) => {
            info!(
                "wolfpack/boot: management SoftAP BSSID {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} (excluded from WiGLE)",
                m[0], m[1], m[2], m[3], m[4], m[5]
            );
            Some(m)
        }
        Err(e) => {
            warn!(
                "wolfpack/boot: could not read SoftAP MAC; own AP may appear in wardrive data: {e:#}"
            );
            None
        }
    };

    let mem_policy = memory_policy::load_control_runtime_policy();
    stats::MAC_TABLE_MAX_ENTRIES.store(mem_policy.mac_table_max_entries as u32, Ordering::Relaxed);
    stats::MAC_TABLE_GEO_SLOTS_MAX
        .store(mem_policy.mac_table_geo_slots_max as u32, Ordering::Relaxed);
    espnow::configure_rx_queue_capacity(espnow::ESPNOW_RX_QUEUE_CAPACITY_CONTROL);
    storage::configure_sd_writer_queue_capacity(mem_policy.sd_writer_queue_capacity);
    info!(
        "wolfpack/memory-policy: {:?} (free_heap={} spiram_free={})",
        mem_policy,
        stats::free_heap_bytes(),
        stats::free_spiram_heap_bytes(),
    );

    Ok(BootNetworking {
        nvs: hw.nvs,
        nvs_arc: hw.nvs_arc,
        cfg: hw.cfg,
        exclusion: hw.exclusion,
        alert_hub: hw.alert_hub,
        sd_io: hw.sd_io,
        gps_hist: hw.gps_hist,
        gps_supervisor: hw.gps_supervisor,
        gps_fix: hw.gps_fix,
        gps_has_fix: hw.gps_has_fix,
        wifi,
        own_management_ap_bssid,
        mem_policy,
    })
}

/// Phase 3 — ESP-NOW + web/SoftAP provisioning + initial denylist/watchlist broadcasts.
fn init_fleet_downlink(net: BootNetworking) -> Result<BootFleet> {
    let mut control_boot_id: u32 = unsafe { esp_idf_sys::esp_random() };
    if control_boot_id == 0 {
        control_boot_id = 1;
    }
    info!("wolfpack/boot: control_boot_id={control_boot_id:#x} (sniffers reset session on change)");

    let sniffers: Arc<Mutex<HashSet<[u8; 6]>>> = Arc::new(Mutex::new(HashSet::new()));
    let cmd_seq = Arc::new(AtomicU32::new(1));

    let web_snapshots = web::web_snapshots_init();
    if let Some(name) = net.cfg.as_ref().wigle_api_name.as_deref() {
        web::web_snapshots_set_wigle_api_name(name);
    }

    let (nvs_defer, nvs_defer_rx) = NvsDeferQueue::pair(8);

    let web = web::start(
        net.nvs_arc.clone(),
        Arc::clone(&net.exclusion.wigle_exclude_manual),
        Arc::clone(&net.exclusion.wigle_exclude_ssids_manual),
        Arc::clone(&net.exclusion.detection_exclude_manual),
        Arc::clone(&net.exclusion.ssid_watchlist),
        Arc::clone(&net.exclusion.denylist_dirty),
        Arc::clone(&net.exclusion.watchlist_dirty),
        Arc::clone(&net.exclusion.detection_masks_dirty),
        nvs_defer.clone(),
        Arc::clone(&net.gps_fix),
        Arc::clone(&sniffers),
        Arc::clone(&web_snapshots),
        control_boot_id,
        Arc::clone(&cmd_seq),
        Arc::clone(&net.exclusion.wigle_home_bssid),
        net.own_management_ap_bssid,
        Arc::clone(&net.sd_io),
    )?;
    info!("wolfpack/boot: HTTP server started on SoftAP");

    info!(
        "wolfpack/boot: espnow rx queue alloc (internal_free={} spiram_free={} slots={} ring_bytes={})",
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
        espnow::rx_queue_capacity(),
        espnow::ESPNOW_RX_QUEUE_RING_BYTES_CONTROL,
    );
    let (rx_tx, rx_rx) = wardriver_shared::spiram_queue::bounded_channel::<espnow::RxFrame>(
        espnow::rx_queue_capacity(),
    );
    espnow::init(rx_tx)?;
    match espnow::refresh_stack_after_wifi(CONTROL_AP_CHANNEL, ESPNOW_PEER_IF) {
        Ok(()) => {
            stats::CONTROL_ESPNOW_STACK_REFRESH_OK.fetch_add(1, Ordering::Relaxed);
            info!("wolfpack/boot: ESP-NOW stack refresh after init ok");
        }
        Err(e) => {
            stats::CONTROL_ESPNOW_STACK_REFRESH_FAIL.fetch_add(1, Ordering::Relaxed);
            warn!("wolfpack/boot: ESP-NOW stack refresh after init failed: {e:#}");
        }
    }

    info!("wolfpack/boot: entering main loop");

    let scanning_allowed = Arc::new(AtomicBool::new(false));

    if let Err(e) = super::fleet_cmd::broadcast_bssid_denylist(
        net.own_management_ap_bssid,
        &net.exclusion.wigle_home_bssid,
        &net.exclusion.wigle_exclude_manual,
        ESPNOW_PEER_IF,
    ) {
        warn!("wolfpack/boot: initial BSSID denylist broadcast: {e:#}");
    }
    if let Err(e) = super::fleet_cmd::broadcast_ssid_denylist(
        &net.exclusion.wigle_exclude_ssids_manual,
        ESPNOW_PEER_IF,
    ) {
        warn!("wolfpack/boot: initial SSID denylist broadcast: {e:#}");
    }
    if let Err(e) =
        super::fleet_cmd::broadcast_ssid_watchlist(&net.exclusion.ssid_watchlist, ESPNOW_PEER_IF)
    {
        warn!("wolfpack/boot: initial SSID watchlist broadcast: {e:#}");
    }

    Ok(BootFleet {
        nvs: net.nvs,
        nvs_arc: net.nvs_arc,
        cfg: net.cfg,
        exclusion: net.exclusion,
        alert_hub: net.alert_hub,
        sd_io: net.sd_io,
        gps_hist: net.gps_hist,
        gps_supervisor: net.gps_supervisor,
        gps_fix: net.gps_fix,
        gps_has_fix: net.gps_has_fix,
        wifi: net.wifi,
        own_management_ap_bssid: net.own_management_ap_bssid,
        mem_policy: net.mem_policy,
        control_boot_id,
        sniffers,
        cmd_seq,
        web_snapshots,
        web,
        nvs_defer,
        nvs_defer_rx,
        rx_rx,
        scanning_allowed,
    })
}

/// Phase 4 — control-loop state, hook registrations, and ingest worker spawn.
fn spawn_workers(fleet: BootFleet) -> Result<BootWorkers> {
    let state = ControlLoopState::new(
        fleet.gps_has_fix,
        &fleet.mem_policy,
        Arc::clone(&fleet.gps_hist),
    );

    super::peer_pool::register_ingest(Arc::clone(&state.ingest));
    super::peer_pool::register_post_dump_flush_hook(super::fleet_cmd::flush_post_dump_downlink);
    super::fleet_cmd::register_control_gps_hooks();
    super::time_sync_rtt::register_control_wall_clock();

    let worker_deps = Arc::new(IngestWorkerDeps {
        control_boot_id: fleet.control_boot_id,
        mem_policy: fleet.mem_policy,
        peer_if: ESPNOW_PEER_IF,
        own_management_ap_bssid: fleet.own_management_ap_bssid,
        sniffers: Arc::clone(&fleet.sniffers),
        nvs: Arc::clone(&fleet.nvs_arc),
        cfg: Arc::clone(&fleet.cfg),
        scanning_allowed: Arc::clone(&fleet.scanning_allowed),
        cmd_seq: Arc::clone(&fleet.cmd_seq),
        wigle_home: Arc::clone(&fleet.exclusion.wigle_home_bssid),
        wigle_manual: Arc::clone(&fleet.exclusion.wigle_exclude_manual),
        wigle_ssids_manual: Arc::clone(&fleet.exclusion.wigle_exclude_ssids_manual),
        detection_ignore: Arc::clone(&fleet.exclusion.detection_exclude_manual),
        gps_hist: Arc::clone(&state.gps_hist),
        gps_fix: Arc::clone(&fleet.gps_fix),
        alert_hub: Arc::clone(&fleet.alert_hub),
        sd_io: Arc::clone(&fleet.sd_io),
    });
    let _workers = control_workers::spawn_control_workers(
        fleet.rx_rx,
        Arc::clone(&state.ingest),
        worker_deps,
    )?;
    info!(
        "wolfpack/boot: worker threads started (espnow-ingest stack_bytes={})",
        stats::CONTROL_INGEST_STACK_BYTES.load(Ordering::Relaxed),
    );

    Ok(BootWorkers {
        nvs: fleet.nvs,
        exclusion: fleet.exclusion,
        alert_hub: fleet.alert_hub,
        sd_io: fleet.sd_io,
        gps_supervisor: fleet.gps_supervisor,
        gps_fix: fleet.gps_fix,
        wifi: fleet.wifi,
        own_management_ap_bssid: fleet.own_management_ap_bssid,
        control_boot_id: fleet.control_boot_id,
        sniffers: fleet.sniffers,
        cmd_seq: fleet.cmd_seq,
        web_snapshots: fleet.web_snapshots,
        _web: fleet.web,
        _nvs_defer: fleet.nvs_defer,
        nvs_defer_rx: fleet.nvs_defer_rx,
        scanning_allowed: fleet.scanning_allowed,
        state,
    })
}
