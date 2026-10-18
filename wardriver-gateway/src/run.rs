//! Gateway main loop: WiFi APSTA + ESP-NOW + PackLink SoftAP TCP + PSRAM spool.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Result;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};
use log::{info, warn};
use wardriver_fleet_realtime::ingest;
use wardriver_fleet_realtime::peer_pool;
use wardriver_fleet_realtime::ESPNOW_PEER_IF;
use wardriver_proto::CONTROL_AP_CHANNEL;
use wardriver_shared::espnow;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;

use crate::espnow_bridge::{self, EspnowBridge};
use crate::gateway_softap;
use crate::ingest_state::GatewayIngestState;
use crate::packlink_nvs_defer::{self, PacklinkNvsDeferQueue};
use crate::packlink_trust;
use crate::policy_downlink::GatewayFleetState;
use crate::policy_store;
use crate::psram_spool;
use crate::softap_transport;

static GATEWAY_BOOT_ID: AtomicU32 = AtomicU32::new(0);
static FLEET: OnceLock<Arc<GatewayFleetState>> = OnceLock::new();
static BRIDGE_TICK: OnceLock<std::sync::mpsc::SyncSender<()>> = OnceLock::new();
static LAST_BRIDGE_FLUSH_MS: AtomicU32 = AtomicU32::new(0);

const BRIDGE_FLUSH_PERIOD_MS: u64 = 500;

static GATEWAY_BOOT_HOLD_LOG_SEC: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

pub fn run_gateway() -> Result<()> {
    info!("wolfPACK gateway v{}", env!("CARGO_PKG_VERSION"));

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;
    let _boot_button = wardriver_shared::boot_button::BootButtonHandle::spawn(pins.gpio28)
        .map_err(|e| {
            warn!("boot_button: {e:#}");
            e
        })
        .ok();
    let _gateway_led = crate::gateway_led::GatewayLedHandle::spawn(pins.gpio27)
        .map_err(|e| {
            warn!("gateway_led: {e:#}");
            e
        })
        .ok();

    let sys_loop = EspSystemEventLoop::take()?;
    let nvs = EspNvsPartition::<NvsDefault>::take()?;
    packlink_trust::bind_nvs(nvs.clone());
    policy_store::bind_nvs(nvs.clone());

    stats::init_boot_time();
    task_wdt::register_current_task_for_watchdog();

    let boot_id = gateway_boot_id();
    psram_spool::init(psram_spool::DEFAULT_SPOOL_CAPACITY)?;

    let fleet = Arc::new(GatewayFleetState::new(boot_id));
    fleet.register_hooks();
    let _ = FLEET.set(fleet.clone());
    peer_pool::register_post_dump_flush(crate::policy_downlink::gateway_post_dump_flush);

    let ingest = Arc::new(Mutex::new(GatewayIngestState::default()));
    let bridge: Arc<Mutex<dyn ingest::IngestPeerPoolState>> =
        Arc::new(Mutex::new(GatewayIngestBridge(ingest.clone())));
    peer_pool::register_ingest(bridge);

    let mut wifi = init_wifi_apsta(peripherals.modem, sys_loop, nvs.clone())?;

    let (frame_tx, frame_rx) =
        wardriver_shared::spiram_queue::bounded_channel::<espnow::RxFrame>(256);
    espnow::init(frame_tx)?;
    match espnow::refresh_stack_after_wifi(CONTROL_AP_CHANNEL, ESPNOW_PEER_IF) {
        Ok(()) => info!("gateway: ESP-NOW stack refresh after wifi ok"),
        Err(e) => warn!("gateway: ESP-NOW stack refresh after wifi failed: {e:#}"),
    }
    espnow::set_channel(CONTROL_AP_CHANNEL)?;
    espnow::add_broadcast_peer(CONTROL_AP_CHANNEL, ESPNOW_PEER_IF)?;
    info!("gateway espnow on channel {CONTROL_AP_CHANNEL}");

    restore_policy_from_nvs(&fleet);

    let tick_tx = EspnowBridge::spawn(frame_rx, fleet.clone(), ingest)?;
    let _ = BRIDGE_TICK.set(tick_tx.clone());

    let (packlink_nvs_defer, packlink_nvs_defer_rx) = PacklinkNvsDeferQueue::pair(4);

    softap_transport::register_fleet(fleet.clone(), move || {
        espnow_bridge::signal_flush(&tick_tx);
    });
    softap_transport::register_nvs_defer(packlink_nvs_defer);
    softap_transport::init_softap_transport()?;

    let _nvs = nvs;

    info!(
        "gateway ready: boot_id={} channel={} spool_cap={} fill={}%",
        boot_id,
        CONTROL_AP_CHANNEL,
        psram_spool::capacity_bytes(),
        psram_spool::fill_percent()
    );

    loop {
        gateway_softap::apply_pending_softap_restart(&mut wifi);
        gateway_softap::evict_stale_softap_clients(softap_transport::phone_connected());
        packlink_nvs_defer::drain(&packlink_nvs_defer_rx);
        policy_store::drain_save();
        softap_transport::maybe_notify_sniffer_status();
        softap_transport::poll_inbound_work();
        if let Some(fleet) = FLEET.get() {
            crate::capture_job::poll(fleet);
        }
        if softap_transport::phone_connected() {
            let now = stats::now_ms();
            let last = u64::from(LAST_BRIDGE_FLUSH_MS.load(Ordering::Relaxed));
            if now.saturating_sub(last) >= BRIDGE_FLUSH_PERIOD_MS {
                LAST_BRIDGE_FLUSH_MS.store(now.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
                if let Some(tick) = BRIDGE_TICK.get() {
                    espnow_bridge::signal_flush(tick);
                }
            }
        }
        tick_gateway_boot_fleet_unbind();
        task_wdt::sleep_feeding_wdt(Duration::from_millis(50));
    }
}

fn tick_gateway_boot_fleet_unbind() {
    let hold_ms = wardriver_shared::boot_button::hold_elapsed_ms();
    if hold_ms > 0 {
        if let Ok(mut g) = GATEWAY_BOOT_HOLD_LOG_SEC.lock() {
            wardriver_shared::boot_button::log_hold_progress_if_due(&mut *g, hold_ms);
        }
    } else if let Ok(mut g) = GATEWAY_BOOT_HOLD_LOG_SEC.lock() {
        *g = None;
    }

    if !wardriver_shared::boot_button::take_fleet_unbind_request() {
        return;
    }
    let Some(fleet) = FLEET.get() else {
        return;
    };
    match crate::policy_downlink::unbind_all_fleet(fleet) {
        Ok(()) => {
            info!("gateway: fleet unbind (button) complete");
            crate::gateway_led::request_success_flash();
        }
        Err(e) => warn!("gateway: fleet unbind (button): {e:#}"),
    }
}

/// NVS policy downlink runs only after WiFi + ESP-NOW are live (apply_policy_sync uses esp_now_send).
fn restore_policy_from_nvs(fleet: &GatewayFleetState) {
    let Some(policy) = policy_store::load_policy() else {
        return;
    };
    if let Err(e) = crate::policy_downlink::apply_policy_sync(fleet, policy) {
        warn!("gateway: restore policy from NVS: {e:#}");
    } else {
        crate::policy_downlink::enqueue_boot_welcomes(fleet);
    }
}

struct GatewayIngestBridge(Arc<Mutex<GatewayIngestState>>);

impl ingest::IngestPeerPoolState for GatewayIngestBridge {
    fn has_incomplete_reasm_for_mac(&self, mac: &[u8; 6]) -> bool {
        self.0
            .lock()
            .ok()
            .map(|g| g.has_incomplete_reasm_for_mac(mac))
            .unwrap_or(true)
    }

    fn drop_state_for_sniffer(&mut self, sniffer_id: u32) {
        if let Ok(mut g) = self.0.lock() {
            g.drop_state_for_sniffer(sniffer_id);
        }
    }
}

fn init_wifi_apsta(
    modem: esp_idf_hal::modem::Modem<'static>,
    sys_loop: EspSystemEventLoop,
    nvs: EspNvsPartition<NvsDefault>,
) -> Result<BlockingWifi<EspWifi<'static>>> {
    let wifi = EspWifi::new(modem, sys_loop.clone(), Some(nvs))?;
    let mut wifi = BlockingWifi::wrap(wifi, sys_loop)?;
    gateway_softap::init_apsta(&mut wifi)?;
    info!("gateway wifi apsta started");
    Ok(wifi)
}

/// Compatibility hook: update the transport-owned phone connection state.
pub fn set_phone_connected(connected: bool) {
    softap_transport::set_phone_connected(connected);
}

/// Fleet RAM state for hooks (hello welcome, gateway_info).
#[must_use]
pub fn fleet_state() -> Option<&'static Arc<GatewayFleetState>> {
    FLEET.get()
}

fn gateway_boot_id() -> u32 {
    let cached = GATEWAY_BOOT_ID.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let mut boot_id: u32 = unsafe { esp_idf_sys::esp_random() };
    if boot_id == 0 {
        boot_id = 1;
    }
    GATEWAY_BOOT_ID.store(boot_id, Ordering::Relaxed);
    info!("gateway: control_boot_id={boot_id:#x} (sniffers reset session on change)");
    boot_id
}

/// Rotate `control_boot_id` so each rostered sniffer detects the session change
/// (from `cmd.control_boot_id` in the re-sent ControlWelcome frames) and re-adds
/// its ESP-NOW peer. Called on phone reconnect before the welcome flush.
pub fn rotate_gateway_boot_id() {
    let mut new_id: u32 = unsafe { esp_idf_sys::esp_random() };
    if new_id == 0 {
        new_id = 1;
    }
    GATEWAY_BOOT_ID.store(new_id, Ordering::Release);
    if let Some(fleet) = FLEET.get() {
        fleet.control_boot_id.store(new_id, Ordering::Release);
    }
    info!("gateway: rotated control_boot_id to {new_id:#x} (phone reconnect)");
}
