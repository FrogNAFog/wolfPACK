//! Control boot: SD mount, GPS UART, SoftAP/STA WiFi, optional upload window.

use std::sync::atomic::Ordering;
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::hal::gpio::AnyIOPin;
use esp_idf_svc::hal::sd::{spi::SdSpiHostDriver, SdCardConfiguration, SdCardDriver};
use esp_idf_svc::hal::spi::{config::DriverConfig, Dma, SpiDriver};
use esp_idf_svc::hal::uart;
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::io::vfs::MountedFatfs;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{AccessPointConfiguration, AuthMethod, ClientConfiguration, Configuration};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::{info, warn};

use crate::config;
use crate::storage;
use crate::uploader;
use wardriver_shared::espnow;
use wardriver_shared::task_wdt;
use wardriver_shared::wifi_tx_power;
use wardriver_stats as stats;
use wardriver_stats::fleet::{
    CONTROL_SOFTAP_GATEWAY, CONTROL_SOFTAP_NETMASK, CONTROL_SOFTAP_SUBNET,
    ESPNOW_MAX_ENCRYPT_PEERS, SOFTAP_MAX_CONNECTIONS,
};

use esp_idf_sys::wifi_interface_t_WIFI_IF_AP as ESPNOW_PEER_IF;

use super::sd_bus_prep;
use super::CONTROL_AP_CHANNEL;

// Board SD + GPS pins: `wardriver_board` profiles selected at build time (`board-marauder` / `board-xiao`).
pub(crate) const GPS_UART_BAUD: u32 = 9600;

/// Boot-time SDSPI mount attempts before reporting failure.
const SD_MOUNT_MAX_ATTEMPTS: u32 = 3;

pub(crate) fn board_profile() -> &'static wardriver_board::BoardProfile {
    wardriver_board::active_profile()
}

pub(crate) fn configure_wifi_country_us() {
    let _rc = wardriver_shared::wifi_regulatory::configure_wifi_country_us();
}

/// Keep the SD mount alive for the program lifetime.
///
/// `MountedFatfs<T>` is generic over the underlying `Fatfs<...>` driver type, which is
/// cumbersome to name in a `static`. We only need to keep it alive (never access it),
/// so we store an erased raw pointer.
static SD_MOUNT_PTR: OnceLock<usize> = OnceLock::new();

/// After STA connect: learn AP BSSID for WiGLE exclusion when API credentials are set.
pub(crate) fn learn_home_wigle_bssid(
    wifi: &BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
    nvs: &EspNvsPartition<NvsDefault>,
    home_out: &Arc<Mutex<Option<[u8; 6]>>>,
) -> Result<()> {
    if !cfg.wigle_configured() {
        return Ok(());
    }
    let ap = wifi
        .wifi()
        .get_ap_info()
        .map_err(|e| anyhow!("get_ap_info: {e:?}"))?;
    let want = cfg.sta_ssid.as_deref().unwrap_or("").trim();
    if want.is_empty() {
        return Ok(());
    }
    if ap.ssid.as_str().trim() != want {
        warn!(
            "wolfpack: associated AP SSID {:?} != home {:?}; not saving wg_home_bssid",
            ap.ssid.as_str(),
            want
        );
        return Ok(());
    }
    let b = ap.bssid;
    config::wigle_home_bssid_save(nvs, &b)?;
    if let Ok(mut g) = home_out.lock() {
        *g = Some(b);
    }
    info!(
        "wolfpack: learned wg_home_bssid {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} for WiGLE exclusion",
        b[0], b[1], b[2], b[3], b[4], b[5]
    );
    Ok(())
}

pub(crate) fn mount_sd_for_board(
    spi: esp_idf_hal::spi::SPI2<'static>,
    sck: impl esp_idf_hal::gpio::OutputPin + 'static,
    mosi: impl esp_idf_hal::gpio::OutputPin + 'static,
    miso: impl esp_idf_hal::gpio::InputPin + 'static,
    cs: impl esp_idf_hal::gpio::OutputPin + 'static,
) -> Result<()> {
    mount_sd_fatfs(spi, sck, mosi, miso, cs)
}

pub(crate) fn open_gps_uart(
    uart: esp_idf_hal::uart::UART1<'static>,
    tx: impl esp_idf_hal::gpio::OutputPin + 'static,
    rx: impl esp_idf_hal::gpio::InputPin + 'static,
    cfg: &uart::config::Config,
) -> Result<uart::UartDriver<'static>> {
    Ok(uart::UartDriver::new(
        uart,
        tx,
        rx,
        Option::<AnyIOPin>::None,
        Option::<AnyIOPin>::None,
        cfg,
    )?)
}

fn mount_sd_fatfs(
    spi: esp_idf_hal::spi::SPI2<'static>,
    sck: impl esp_idf_hal::gpio::OutputPin + 'static,
    mosi: impl esp_idf_hal::gpio::OutputPin + 'static,
    miso: impl esp_idf_hal::gpio::InputPin + 'static,
    cs: impl esp_idf_hal::gpio::OutputPin + 'static,
) -> Result<()> {
    let bp = board_profile();
    let pin_nums = (bp.sd_sck, bp.sd_mosi, bp.sd_miso, bp.sd_cs);

    sd_bus_prep::prep_and_settle(bp);
    task_wdt::reset_if_subscribed();

    let mut last_err = anyhow!("sd mount failed");

    match try_mount_once(spi, sck, mosi, miso, cs) {
        Ok(()) => {
            stats::record_sd_mount_success(1);
            return Ok(());
        }
        Err(e) => {
            warn!("wolfpack/sd: mount attempt 1/{SD_MOUNT_MAX_ATTEMPTS} failed: {e:#}");
            last_err = e;
        }
    }

    for attempt in 2..=SD_MOUNT_MAX_ATTEMPTS {
        sd_bus_prep::prep_and_settle(bp);
        task_wdt::reset_if_subscribed();
        // SAFETY: SD pins are exclusive to mount; the previous driver stack was fully dropped.
        let spi = unsafe { esp_idf_hal::spi::SPI2::steal() };
        let sck_pin = unsafe { AnyIOPin::steal(pin_nums.0) };
        let mosi_pin = unsafe { AnyIOPin::steal(pin_nums.1) };
        let miso_pin = unsafe { AnyIOPin::steal(pin_nums.2) };
        let cs_pin = unsafe { AnyIOPin::steal(pin_nums.3) };
        let mount_result = try_mount_once(spi, sck_pin, mosi_pin, miso_pin, cs_pin);
        match mount_result {
            Ok(()) => {
                info!("wolfpack/sd: mount OK on attempt {attempt}/{SD_MOUNT_MAX_ATTEMPTS}");
                stats::record_sd_mount_success(attempt);
                return Ok(());
            }
            Err(e) => {
                warn!("wolfpack/sd: mount attempt {attempt}/{SD_MOUNT_MAX_ATTEMPTS} failed: {e:#}");
                last_err = e;
            }
        }
    }

    stats::record_sd_mount_failed(SD_MOUNT_MAX_ATTEMPTS);
    Err(last_err)
}

fn try_mount_once(
    spi: esp_idf_hal::spi::SPI2<'static>,
    sck: impl esp_idf_hal::gpio::OutputPin + 'static,
    mosi: impl esp_idf_hal::gpio::OutputPin + 'static,
    miso: impl esp_idf_hal::gpio::InputPin + 'static,
    cs: impl esp_idf_hal::gpio::OutputPin + 'static,
) -> Result<()> {
    let spi_driver = SpiDriver::new(
        spi,
        sck,
        mosi,
        Some(miso),
        &DriverConfig::default().dma(Dma::Auto(4096)),
    )?;
    let sd_card_driver = SdCardDriver::new_spi(
        SdSpiHostDriver::new(
            spi_driver,
            Some(cs),
            AnyIOPin::none(),
            AnyIOPin::none(),
            AnyIOPin::none(),
            None,
        )?,
        &SdCardConfiguration::new(),
    )?;
    let mounted = MountedFatfs::mount(
        Fatfs::new_sdcard(0, sd_card_driver)?,
        storage::MOUNT_POINT,
        4,
    )?;
    // Keep the mount alive for program lifetime so the VFS registration stays valid.
    let ptr = Box::into_raw(Box::new(mounted)) as usize;
    let _ = SD_MOUNT_PTR.set(ptr);
    Ok(())
}

/// SoftAP interface MAC; matches beacon BSSID for our management AP.
pub(crate) fn read_softap_mac() -> Result<[u8; 6]> {
    let mut mac = [0u8; 6];
    unsafe {
        let rc = esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_SOFTAP,
        );
        if rc != 0 {
            return Err(anyhow!("esp_read_mac(SOFTAP) rc={rc}"));
        }
    }
    Ok(mac)
}

/// SoftAP used for the dashboard; fixed channel for ESP-NOW alignment.
fn control_ap_configuration(cfg: &config::Config) -> Result<AccessPointConfiguration> {
    let ssid_s = cfg.resolved_ap_ssid();
    let pass_s = cfg.resolved_ap_password();
    Ok(AccessPointConfiguration {
        ssid: ssid_s
            .try_into()
            .map_err(|_| anyhow!("control AP SSID is too long"))?,
        ssid_hidden: false,
        channel: CONTROL_AP_CHANNEL,
        auth_method: AuthMethod::WPA2Personal,
        password: pass_s
            .try_into()
            .map_err(|_| anyhow!("control AP password is too long"))?,
        max_connections: SOFTAP_MAX_CONNECTIONS,
        ..Default::default()
    })
}

pub(crate) fn start_control_ap(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
) -> Result<()> {
    crate::wifi_ap_tune::prepare_control_softap_before_config()?;
    let ap = control_ap_configuration(cfg)?;
    let mixed = Configuration::Mixed(Default::default(), ap);
    wifi.set_configuration(&mixed)?;
    wifi.start()?;
    crate::wifi_ap_tune::configure_control_ap_protocols();
    info!(
        "wolfpack/boot: management SoftAP ch={} max_connections={} (ESPNOW encrypt peers={})",
        CONTROL_AP_CHANNEL, SOFTAP_MAX_CONNECTIONS, ESPNOW_MAX_ENCRYPT_PEERS
    );
    Ok(())
}

/// Last DHCP lease octet on the fleet subnet; pool runs `.2` through this value inclusive.
const DHCP_POOL_START_OCTET: u8 = 2;
const DHCP_POOL_END_OCTET: u8 = 254;

/// Mirrors ESP-IDF `dhcps_lease_t` (not in esp-idf-sys bindings).
#[repr(C)]
struct DhcpsLease {
    enable: bool,
    start_ip: esp_idf_sys::esp_ip4_addr_t,
    end_ip: esp_idf_sys::esp_ip4_addr_t,
}

fn softap_addr(last_octet: u8) -> [u8; 4] {
    let mut ip = CONTROL_SOFTAP_SUBNET;
    ip[3] = last_octet;
    ip
}

fn ip4_set(addr: &mut esp_idf_sys::esp_ip4_addr_t, ip: [u8; 4]) {
    unsafe {
        esp_idf_sys::esp_netif_set_ip4_addr(addr, ip[0], ip[1], ip[2], ip[3]);
    }
}

fn fmt_ipv4(octets: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", octets[0], octets[1], octets[2], octets[3])
}

/// Pin management SoftAP to [`CONTROL_SOFTAP_GATEWAY`]/24 and enable DHCP for operator clients.
///
/// Sniffers do not join the SoftAP (fleet path is ESP-NOW only). One SoftAP association
/// slot is reserved for the operator phone/laptop dashboard.
///
/// Call only after [`crate::provisioning::wait_ap_netif_ready`].
pub(crate) fn configure_management_softap_netif(
    wifi: &BlockingWifi<EspWifi<'static>>,
) -> Result<()> {
    let netif = wifi.wifi().ap_netif().handle();

    let mut ip_info = esp_idf_sys::esp_netif_ip_info_t {
        ip: esp_idf_sys::esp_ip4_addr_t { addr: 0 },
        netmask: esp_idf_sys::esp_ip4_addr_t { addr: 0 },
        gw: esp_idf_sys::esp_ip4_addr_t { addr: 0 },
    };
    ip4_set(&mut ip_info.ip, CONTROL_SOFTAP_GATEWAY);
    ip4_set(&mut ip_info.netmask, CONTROL_SOFTAP_NETMASK);
    ip4_set(&mut ip_info.gw, CONTROL_SOFTAP_GATEWAY);

    unsafe {
        let rc = esp_idf_sys::esp_netif_dhcps_stop(netif);
        if rc != esp_idf_sys::ESP_OK {
            warn!("wolfpack/net: dhcps_stop rc={rc} (continuing)");
        }

        let rc = esp_idf_sys::esp_netif_set_ip_info(netif, &ip_info);
        if rc != esp_idf_sys::ESP_OK {
            return Err(anyhow!("esp_netif_set_ip_info rc={rc}"));
        }

        let mut lease = DhcpsLease {
            enable: true,
            start_ip: esp_idf_sys::esp_ip4_addr_t { addr: 0 },
            end_ip: esp_idf_sys::esp_ip4_addr_t { addr: 0 },
        };
        ip4_set(&mut lease.start_ip, softap_addr(DHCP_POOL_START_OCTET));
        ip4_set(&mut lease.end_ip, softap_addr(DHCP_POOL_END_OCTET));

        let rc = esp_idf_sys::esp_netif_dhcps_option(
            netif,
            esp_idf_sys::esp_netif_dhcp_option_mode_t_ESP_NETIF_OP_SET,
            esp_idf_sys::esp_netif_dhcp_option_id_t_ESP_NETIF_REQUESTED_IP_ADDRESS,
            &mut lease as *mut _ as *mut core::ffi::c_void,
            core::mem::size_of::<DhcpsLease>() as u32,
        );
        if rc != esp_idf_sys::ESP_OK {
            warn!("wolfpack/net: dhcps lease pool rc={rc}");
        }

        let rc = esp_idf_sys::esp_netif_dhcps_start(netif);
        if rc != esp_idf_sys::ESP_OK {
            warn!("wolfpack/net: dhcps_start rc={rc}");
        }
    }

    info!(
        "wolfpack/net: SoftAP {} DHCP .{}.-.{} (operator clients; sniffers ESP-NOW only)",
        fmt_ipv4(CONTROL_SOFTAP_GATEWAY),
        DHCP_POOL_START_OCTET,
        DHCP_POOL_END_OCTET,
    );
    Ok(())
}

/// Disconnect STA (if any) and re-apply the management SoftAP on [`CONTROL_AP_CHANNEL`]
/// so ESP-NOW peers align again after a WiGLE upload window.
pub(crate) fn restore_control_softap_home_channel(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
) {
    let _ = wifi.disconnect();
    if let Err(e) = start_control_ap(wifi, cfg) {
        warn!(
            "wolfpack: could not re-apply SoftAP on channel {}: {e:#}",
            CONTROL_AP_CHANNEL
        );
    }
    if let Err(e) = crate::provisioning::wait_ap_netif_ready(wifi, Duration::from_secs(10)) {
        warn!("wolfpack: SoftAP netif after home channel restore: {e:#}");
    } else if let Err(e) = configure_management_softap_netif(wifi) {
        warn!("wolfpack: SoftAP IP/DHCP after home channel restore: {e:#}");
    }
    wifi_tx_power::apply_fleet_cap();

    stats::CONTROL_WIFI_SOFTAP_RESTORE_COUNT.fetch_add(1, Ordering::Relaxed);
    // Upload window runs before `espnow::init`; skip refresh until RX queue sender is wired.
    if espnow::rx_sender_wired() {
        match espnow::refresh_stack_after_wifi(CONTROL_AP_CHANNEL, ESPNOW_PEER_IF) {
            Ok(()) => {
                stats::CONTROL_ESPNOW_STACK_REFRESH_OK.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                stats::CONTROL_ESPNOW_STACK_REFRESH_FAIL.fetch_add(1, Ordering::Relaxed);
                warn!("wolfpack: ESP-NOW stack refresh after SoftAP restore failed: {e:#}");
            }
        }
    }
}

struct UploadOledThrottle {
    last_ms: u64,
    last_pct: u8,
}

static UPLOAD_OLED_THROTTLE: Mutex<UploadOledThrottle> = Mutex::new(UploadOledThrottle {
    last_ms: 0,
    last_pct: 255,
});

fn reset_upload_oled_throttle() {
    if let Ok(mut t) = UPLOAD_OLED_THROTTLE.lock() {
        t.last_ms = 0;
        t.last_pct = 255;
    }
}

fn upload_report_progress(
    hub: &crate::alert_feedback::AlertHub,
    p: crate::uploader::UploadProgress,
) {
    use wardriver_stats::upload_progress::UploadProgressKind;

    // Throttle byte-% updates; other phases always refresh immediately.
    let should_draw = match p.kind {
        UploadProgressKind::Uploading { byte_pct, .. } => {
            let now = stats::now_ms();
            let mut t = UPLOAD_OLED_THROTTLE
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let elapsed = now.saturating_sub(t.last_ms);
            let pct_jump = byte_pct.abs_diff(t.last_pct);
            if elapsed >= 500 || pct_jump >= 5 {
                t.last_ms = now;
                t.last_pct = byte_pct;
                true
            } else {
                false
            }
        }
        _ => true,
    };
    if should_draw {
        let (title, sub) = crate::uploader::upload_oled_message(&p);
        hub.show_oled_status(title.as_str(), Some(sub.as_str()));
    }
}

/// Dedicated pthread stack for mbedTLS multipart upload (keeps main task headroom).
const UPLOAD_THREAD_STACK: usize = 28_672;

/// Boot-time upload window: `upload_on_boot` plus at least one destination credential.
pub(crate) fn boot_upload_configured(cfg: &config::Config) -> bool {
    cfg.upload_on_boot && (cfg.wigle_configured() || cfg.wdgwars_configured())
}

fn sta_client_configuration(cfg: &config::Config) -> Result<ClientConfiguration> {
    let ssid = cfg
        .sta_ssid
        .as_deref()
        .ok_or_else(|| anyhow!("missing sta_ssid"))?;
    let pass = cfg.sta_password.as_deref().unwrap_or("");
    let auth = if pass.is_empty() {
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };
    Ok(ClientConfiguration {
        ssid: ssid
            .try_into()
            .map_err(|_| anyhow!("sta_ssid is too long for ESP-IDF WiFi config"))?,
        bssid: None,
        auth_method: auth,
        password: pass
            .try_into()
            .map_err(|_| anyhow!("sta_password is too long for ESP-IDF WiFi config"))?,
        channel: None,
        ..Default::default()
    })
}

/// STA only — tears down SoftAP; use before upload, then restore via [`restore_control_softap_home_channel`].
pub(crate) fn connect_sta_only(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    cfg: &config::Config,
) -> Result<()> {
    const STA_CONNECT_WAIT: Duration = Duration::from_secs(15);
    const POLL: Duration = Duration::from_millis(200);

    task_wdt::reset_if_subscribed();
    let client = sta_client_configuration(cfg)?;
    wifi.set_configuration(&Configuration::Client(client))?;
    task_wdt::reset_if_subscribed();
    wifi.start()?;
    task_wdt::reset_if_subscribed();
    wifi.wifi_mut().connect()?;
    let connect_deadline = std::time::Instant::now() + STA_CONNECT_WAIT;
    while std::time::Instant::now() < connect_deadline {
        task_wdt::reset_if_subscribed();
        if wifi.wifi().is_connected()? {
            break;
        }
        task_wdt::sleep_feeding_wdt(POLL);
    }
    if !wifi.wifi().is_connected()? {
        anyhow::bail!("STA connect timed out after {STA_CONNECT_WAIT:?}");
    }
    task_wdt::reset_if_subscribed();
    let netif_deadline = std::time::Instant::now() + STA_CONNECT_WAIT;
    while std::time::Instant::now() < netif_deadline {
        task_wdt::reset_if_subscribed();
        if wifi.is_up()? {
            wifi_tx_power::apply_upload_cap();
            task_wdt::reset_if_subscribed();
            return Ok(());
        }
        task_wdt::sleep_feeding_wdt(POLL);
    }
    anyhow::bail!("STA netif up timed out after {STA_CONNECT_WAIT:?}")
}

/// Run `upload_pending` after STA is up; updates OLED progress and UploadStart/Done alerts.
pub(crate) fn control_upload_pending(
    hub: Arc<crate::alert_feedback::AlertHub>,
    cfg: &config::Config,
    paths: &storage::Paths,
    sd_io: Arc<storage::SdIoHandle>,
    log_label: &str,
    is_boot: bool,
) {
    // Manual upload from dashboard skips this; boot path already gates on NVS prefs.
    if is_boot && !boot_upload_configured(cfg) {
        return;
    }
    // Avoid SNTP, alerts, and the upload thread when there is nothing to send.
    match uploader::pending_has_uploadable_file(sd_io.as_ref(), paths) {
        Ok(false) => {
            info!("uploader: no pending capture files; skipping");
            return;
        }
        Ok(true) => {}
        Err(e) => {
            warn!("{log_label}: pending check: {e:#}");
            return;
        }
    }

    stats::reset_upload_run_counters();
    reset_upload_oled_throttle();
    if !crate::sntp_sync::ensure_wall_clock_for_upload(log_label) {
        stats::clear_upload_phase();
        return;
    }

    stats::set_upload_phase(stats::UPLOAD_PHASE_RUNNING);
    hub.trigger(crate::alert_feedback::AlertKind::UploadStart, None);
    let hub_progress = Arc::clone(&hub);
    let cfg = cfg.clone();
    let paths_owned = paths.clone();
    let sd_io_upload = Arc::clone(&sd_io);
    let (result_tx, result_rx) = sync_channel::<Result<uploader::UploadSummary>>(1);
    let upload_thread = thread::Builder::new()
        .name("https-upload".into())
        .stack_size(UPLOAD_THREAD_STACK)
        .spawn(move || {
            let r = uploader::upload_pending_with_progress(
                &cfg,
                &paths_owned,
                sd_io_upload,
                Some(move |p| {
                    upload_report_progress(&hub_progress, p);
                }),
            );
            let _ = result_tx.send(r);
        });

    let upload_result = match upload_thread {
        Err(e) => Err(anyhow!("spawn https-upload thread: {e}")),
        Ok(_handle) => loop {
            task_wdt::reset_if_subscribed();
            match result_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(r) => break r,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    break Err(anyhow!("https-upload thread disconnected"));
                }
            }
        },
    };

    // Refresh pending CSV count once after upload (boot or manual).
    if stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        storage::refresh_upload_pending_csv_count(sd_io.as_ref());
    }

    match upload_result {
        Ok(summary) => {
            let (title, sub) = uploader::upload_oled_done_message(&summary);
            hub.show_oled_status(&title, Some(&sub));
            stats::set_upload_oled_hold_ms(10_000);
            hub.trigger_upload_done(&title, &sub);
        }
        Err(e) => {
            warn!("{log_label}: upload_pending: {e:#}");
            crate::uploader::log_upload_run_failure(log_label, &e);
            stats::set_upload_phase(stats::UPLOAD_PHASE_FAILED);
            hub.show_oled_status("UPLOAD ERROR", Some("failed"));
            stats::set_upload_oled_hold_ms(10_000);
            hub.trigger_upload_done("UPLOAD ERROR", "failed");
        }
    }
    stats::clear_upload_phase();
}
