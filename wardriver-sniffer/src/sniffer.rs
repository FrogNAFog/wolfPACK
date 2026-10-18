//! Sniffer-node entry point (scan + ESP-NOW report) for ESP-NOW v1.
//!
//! This role does **not** run an HTTP server, SoftAP, or management UI. Fleet setup and
//! `/` / `/setup` live only on the **control** image (`feature = "control"`). Sniffers are
//! commanded exclusively over ESP-NOW from the control node.
//!
//! **No SD card / no WiGLE CSV:** the `storage` module and FAT mount are omitted from this
//! build (`main.rs` gates them on `feature = "control"`). Only the control node writes
//! `/sdcard/wigle/pending/*.csv` (WiGLE-format rows).
//!
//! Behavior:
//! - Initialize WiFi (STA, no association) and ESP-NOW
//! - Wait for `ControlCmdV1::StartScanning` from control
//! - Capture assigned channels in one pass using **WiFi promiscuous mode** (beacon / probe
//!   response only → `ApEntryV1`), then return to the control channel once and send all
//!   `ScanFrag` frames for that sweep (fewer channel hops than per-channel send)
//! - `dwell_ms` from control is **on-air listen time per channel** after `set_channel` returns
//!   (see [`crate::wifi_promisc::PromiscCapture::capture_channel`]); driver hop time is extra
//!   wall-clock. Control-timeout math uses listen sum plus a per-hop switch budget
//!   ([`proto::SNIFFER_SWEEP_SET_CHANNEL_BUDGET_MS_PER_HOP`]) and measured averages when available.
//! - **Dwell time semantics:** dwell time is applied **per channel** as requested by the control
//!   node. Total sweep duration formula: `total_sweep = n_channels × dwell_ms + hop_overhead`,
//!   where `n_channels` is the number of channels in the sniffer's assigned bucket and
//!   `hop_overhead` accounts for WiFi driver channel-switch latency. Beacon sampling delivers
//!   one capture window per channel; timing includes driver hop overhead between channels.
//!   Sweep wall deviation >20% is logged at warn level.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::time::Duration;
use wardriver_core::ssid_watch::SsidWatch;

use anyhow::Result;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, Configuration, EspWifi};
use log::info;
use log::warn;

use crate::sniffer_ble;
use crate::sniffer_config::SnifferConfig;
use crate::sniffer_dispatch;
use crate::sniffer_espnow::{
    flush_pending_time_set_standalone, send_ack, sniffer_id_from_mac, SnifferSession, RECON_SESSION,
};
use crate::sniffer_led::{self, SnifferLedHandle};
use crate::sniffer_link::{send_boot_hello, SnifferLinkCaps, SnifferLinkState};
use crate::sniffer_outbound_bulk::OutboundBulkStore;
use crate::sniffer_recon::{self, ReconChannelState};
use crate::sniffer_sweep::{run_wifi_sweep_finish, run_wifi_sweep_hop, ChannelSchedule};
use crate::sniffer_wifi::{
    configure_wifi_regulatory, probe_five_ghz_capable, set_sniffer_five_ghz_capable,
    sniffer_boot_id, try_set_channel_with_recovery,
};
use crate::wifi_promisc;
use wardriver_proto::{self as proto, SnifferRole, SnifferState};
use wardriver_shared::espnow;
use wardriver_shared::memory_policy;
use wardriver_stats as stats;

pub fn run_sniffer() -> Result<()> {
    info!("esp32-wardriver SNIFFER v{}", env!("CARGO_PKG_VERSION"));
    info!("sniffer: sniffer_boot_id={:#x}", sniffer_boot_id());

    // NVS must be initialized before WiFi starts so PHY calibration data can load.
    // (IDF will log and fall back to full calibration otherwise; on some setups this can
    // destabilize boot / contribute to watchdog resets.) NVS is owned here (not in the
    // init struct) because the session borrows it; it must outlive the main loop.
    let nvs = EspNvsPartition::<NvsDefault>::take()?;
    info!("wolfpack/boot: NVS partition");

    let mut init = init_sniffer(&nvs)?;
    sniffer_main_loop(&mut init, &nvs)?;
    sniffer_shutdown(&mut init);
    Ok(())
}

/// Owned handles created by the sniffer boot phase and carried into the main loop.
///
/// The boot phase runs the original inline sequence verbatim (same order, same log lines)
/// and returns these handles. `nvs` is deliberately **not** a field here: it is created by
/// [`run_sniffer`] and passed by reference so the loop can keep it alive alongside this
/// struct (a struct that borrows `nvs` and is itself the sole owner would be self-referential).
/// Likewise `recon_cap` (a `PromiscCapture` that borrows `promisc_rx`) is a main-loop local,
/// not a field, for the same reason.
struct SnifferInit<'a> {
    wifi: BlockingWifi<EspWifi<'static>>,
    health_led: SnifferLedHandle,
    _boot_button: Option<wardriver_shared::boot_button::BootButtonHandle>,
    rx_rx: wardriver_shared::spiram_queue::Receiver<espnow::RxFrame>,
    promisc_rx: wardriver_shared::spiram_queue::Receiver<wifi_promisc::PromiscRecord>,
    outbound_bulk: OutboundBulkStore,
    link_caps: SnifferLinkCaps,
    link: SnifferLinkState,
    reconnect_grace_until_ms: u64,
    prev_scan_only: bool,
    recon: ReconChannelState,
    session: SnifferSession<'a>,
    seq: u32,
    scan_batch_id: u32,
    next_channel_idx: usize,
    schedule_sig_prev: u64,
    last_on_control_ms: u64,
    sniffer_id: u32,
}

/// Sniffer boot phase: hardware + WiFi + ESP-NOW + promisc + outbound + config + session.
///
/// Runs the original inline boot sequence verbatim (same order, same log lines) and returns
/// the handles the main loop consumes. NVS is taken by the caller and passed by reference.
fn init_sniffer<'a>(nvs: &'a EspNvsPartition<NvsDefault>) -> Result<SnifferInit<'a>> {
    // If we stop hearing from control for too long, go silent until control speaks again.
    //
    // IMPORTANT: scanning spends time off the control channel. This timeout must be long enough
    // to cover a full capture phase (listen time **plus** WiFi driver hop overhead when applicable),
    // otherwise healthy sniffers can self-silence in dense schedules.
    let peripherals = Peripherals::take()?;
    info!("wolfpack/boot: peripherals taken");
    let pins = peripherals.pins;
    let ledc = peripherals.ledc;
    let health_led = SnifferLedHandle::spawn(ledc.channel0, ledc.timer0, pins.gpio27)?;
    info!(
        "wolfpack/boot: health LED on GPIO{}",
        sniffer_led::XIAO_USER_LED_GPIO
    );
    let _boot_button = wardriver_shared::boot_button::BootButtonHandle::spawn(pins.gpio28)
        .map_err(|e| {
            warn!("boot_button: {e:#}");
            e
        })
        .ok();
    let sys_loop = EspSystemEventLoop::take()?;
    info!("wolfpack/boot: system event loop");

    // Start WiFi driver (STA mode is fine; we won't connect).
    info!("wolfpack/boot: EspWifi::new + start");
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), None)?,
        sys_loop,
    )?;
    // WiFi config can persist in NVS across flashes. If this chip previously ran the
    // control image (SoftAP), `start()` without `set_configuration` may resurrect AP mode.
    // Force a minimal STA-only configuration before starting the driver.
    wifi.set_configuration(&Configuration::Client(Default::default()))?;
    wifi.start()?;
    info!("wolfpack/boot: WiFi STA driver started");
    wardriver_shared::wifi_tx_power::apply_default_cap_once();

    // Centralized regulatory setup
    configure_wifi_regulatory();

    // Load sniffer config (control MAC learned via discovery).
    let mut cfg = SnifferConfig::load(&nvs)?;
    info!("wolfpack/boot: sniffer config loaded");

    // ESP-NOW RX into channel.
    info!("wolfpack/boot: esp_now init");
    let mem_policy = memory_policy::load_runtime_policy();
    espnow::configure_rx_queue_capacity(mem_policy.espnow_rx_queue_capacity);
    let (rx_tx, rx_rx) = wardriver_shared::spiram_queue::bounded_channel::<espnow::RxFrame>(
        espnow::rx_queue_capacity(),
    );
    espnow::init(rx_tx)?;

    wifi_promisc::configure_promisc_queue_capacity(mem_policy.promisc_queue_capacity);
    let promisc_cap = wifi_promisc::promisc_queue_capacity();
    let record_bytes = std::mem::size_of::<wifi_promisc::PromiscRecord>();
    info!(
        "wolfpack/boot: promisc RX queue alloc (internal_free={} spiram_free={} slots={} ring_bytes={})",
        stats::free_internal_heap_bytes(),
        stats::free_spiram_heap_bytes(),
        promisc_cap,
        promisc_cap.saturating_mul(record_bytes),
    );
    let (promisc_tx, promisc_rx) =
        wardriver_shared::spiram_queue::bounded_channel::<wifi_promisc::PromiscRecord>(promisc_cap);
    wifi_promisc::set_sender(promisc_tx)?;

    info!(
        "wolfpack/boot: outbound bulk wifi={} probe={} ble={} B (total {} KiB)",
        memory_policy::OUTBOUND_BULK_WIFI_BUDGET_BYTES,
        memory_policy::OUTBOUND_BULK_PROBE_BUDGET_BYTES,
        memory_policy::OUTBOUND_BULK_BLE_BUDGET_BYTES,
        memory_policy::OUTBOUND_BULK_BUDGET_BYTES / 1024,
    );
    let outbound_bulk = OutboundBulkStore::new_from_policy();

    // Start in control channel, listen for broadcast control cmds to learn the control MAC.
    let mut control_mac = cfg.control_mac;
    if control_mac.is_none() {
        control_mac = cfg.bound_control_mac;
    }
    let schedule = ChannelSchedule::default();
    espnow::set_channel(schedule.control_channel)?;

    espnow::add_broadcast_peer(
        schedule.control_channel,
        esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
    )?;
    info!("wolfpack/boot: esp_now peers ready; entering main loop");

    if let Some(mac) = control_mac {
        if let Err(e) = espnow::add_peer(
            mac,
            schedule.control_channel,
            esp_idf_sys::wifi_interface_t_WIFI_IF_STA,
        ) {
            // A stale saved control MAC should not brick boot. We'll still listen for
            // broadcast control frames and relearn control when it appears.
            warn!("sniffer: add_peer(saved control_mac) failed: {e:#}");
            control_mac = None;
            cfg.control_mac = None;
            let _ = cfg.save(&nvs);
        }
    } else {
        info!(
            "sniffer: waiting for control discovery on channel {}",
            schedule.control_channel
        );
    }

    let sniffer_id = sniffer_id_from_mac()?;
    let state = SnifferState::Idle;
    let last_cmd_seq: u32 = 0;

    // Capability probe at boot: non-DFS 5 GHz tune OK (ch 36).
    let five_ghz_cap = probe_five_ghz_capable();
    set_sniffer_five_ghz_capable(five_ghz_cap);
    let _ = espnow::set_channel(schedule.control_channel);

    // Handshake gate: keep `control_mac` as a hint, but don't transmit until we
    // have actually heard a valid control frame after boot.
    let control_established = false;
    let last_control_rx_ms: Option<u64> = None;
    let last_control_boot_id: Option<u32> = None;
    let last_time_set_seq: u32 = 0;
    let pending_time_set: Option<proto::TimeSetV1> = None;
    let last_rtt_time_set_ms: u64 = 0;

    let seq: u32 = 1;
    let scan_batch_id: u32 = 1;
    let next_channel_idx: usize = 0;
    let schedule_sig_prev: u64 = 0;
    let last_on_control_ms: u64 = wardriver_stats::now_ms();
    // BSSIDs/SSIDs excluded from scan reports (synced from control via ESP-NOW).
    let bssid_denylist: HashSet<[u8; 6]> = HashSet::new();
    let ssid_denylist: HashSet<String> = HashSet::new();
    let ssid_watch: SsidWatch = SsidWatch::new();
    // Probe-request wardrive batches are sent only for MACs that raised DetectionAlert this boot.
    let alerted_probe_srcs: HashSet<[u8; 6]> = HashSet::new();

    let pending_ack: bool = false;
    let fleet_slot: u8 = 0;
    let link = SnifferLinkState::new();
    let link_caps = SnifferLinkCaps {
        five_ghz_capable: five_ghz_cap,
        dfs_capable: five_ghz_cap,
    };

    let boot_ms = wardriver_stats::now_ms();
    let reconnect_grace_until_ms = boot_ms.saturating_add(sniffer_led::BOOT_RECONNECT_GRACE_MS);

    send_boot_hello(sniffer_id, &cfg, link_caps);

    let recon = ReconChannelState::new();
    let prev_scan_only = false;

    wardriver_shared::task_wdt::register_current_task_for_watchdog();
    info!("sniffer: task watchdog registered (main loop)");

    // Owned session state for the main loop: constructed once, then borrowed by each sub-tick
    // via `as_ctx()` (and read directly as `session.<field>` outside sub-ticks).
    let session = SnifferSession {
        nvs,
        cfg,
        control_mac,
        schedule,
        state,
        last_cmd_seq,
        control_established,
        last_control_rx_ms,
        last_control_boot_id,
        sniffer_id,
        bssid_denylist,
        ssid_denylist,
        ssid_watch,
        alerted_probe_srcs,
        last_time_set_seq,
        pending_time_set,
        last_rtt_time_set_ms,
        pending_ack,
        fleet_slot,
    };
    let sniffer_id = session.sniffer_id;

    Ok(SnifferInit {
        wifi,
        health_led,
        _boot_button,
        rx_rx,
        promisc_rx,
        outbound_bulk,
        link_caps,
        link,
        reconnect_grace_until_ms,
        prev_scan_only,
        recon,
        session,
        seq,
        scan_batch_id,
        next_channel_idx,
        schedule_sig_prev,
        last_on_control_ms,
        sniffer_id,
    })
}

/// Sniffer main-loop phase: the original infinite control loop, statement-for-statement.
///
/// The loop is `loop { ... }` with no post-loop code in the original (it runs until reset),
/// so this function never returns under normal operation. `recon_cap` is a local here (not a
/// `SnifferInit` field) because it borrows `init.promisc_rx`; it starts `None` and is filled
/// by `run_recon_tick` exactly as in the original.
fn sniffer_main_loop<'a, 'b>(
    init: &'b mut SnifferInit<'a>,
    _nvs: &'a EspNvsPartition<NvsDefault>,
) -> Result<()> {
    let mut recon_cap: Option<wifi_promisc::PromiscCapture<'b>> = None;

    loop {
        wardriver_shared::task_wdt::reset_if_subscribed();
        let now_ms = wardriver_stats::now_ms();
        wardriver_shared::cpu_load::poll(now_ms);

        if init.session.cfg.scan_only() {
            if !init.prev_scan_only {
                init.outbound_bulk.clear();
                init.session.alerted_probe_srcs.clear();
                info!("sniffer: scan-only entered");
            }
            init.prev_scan_only = true;
        } else {
            init.prev_scan_only = false;
        }

        {
            let mut ctx = init.session.as_ctx();
            ctx.drain_rx(&init.rx_rx)?;
        }

        // RECOVERY PATH: without this, link-dead sniffers can never re-join (see plan: Gateway Phone Disconnect / Sniffer Hello Loop Dead).
        init.link.tick_discovery_hello(
            now_ms,
            init.session.control_established,
            init.sniffer_id,
            &init.session.cfg,
            init.link_caps,
        );

        if crate::sniffer_espnow::FLEET_FAST_JOIN_RESET
            .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            init.link.reset_fast_join(now_ms);
        }

        if wardriver_shared::boot_button::take_fleet_unbind_request() {
            info!("sniffer: fleet unbind (button)");
            crate::sniffer_link::apply_local_fleet_unbind(
                &mut init.session.cfg,
                init.session.nvs,
                &mut init.session.control_mac,
                &mut init.session.control_established,
                &mut init.session.last_control_boot_id,
                &mut init.session.fleet_slot,
            );
            init.link.reset_fast_join(now_ms);
            init.reconnect_grace_until_ms =
                now_ms.saturating_add(sniffer_led::BOOT_RECONNECT_GRACE_MS);
        }

        if init.session.state != SnifferState::ReconCapturing {
            if let Some(c) = recon_cap.take() {
                let _ = c.end(init.wifi.wifi_mut());
            }
        }

        if let Some(crate::sniffer_link::WatchdogAction::LinkDead) = init.link.tick_watchdog(
            now_ms,
            init.session.schedule.control_channel,
            init.session.last_control_rx_ms,
            init.session.control_established,
            &mut init.session.pending_ack,
            init.session.nvs,
            &mut init.session.cfg,
            &mut init.session.control_mac,
            &mut init.session.last_control_boot_id,
            &mut init.session.control_established,
        ) {
            if init.session.state != SnifferState::ReconCapturing {
                init.session.state = SnifferState::Idle;
                let _ = RECON_SESSION.lock().map(|mut g| *g = None);
                init.recon.reset();
            }
        }

        if init.link.take_reconnect_grace_bump() {
            init.reconnect_grace_until_ms =
                now_ms.saturating_add(sniffer_led::BOOT_RECONNECT_GRACE_MS);
        }

        init.health_led.set_health(sniffer_led::evaluate_health(
            &init.session.cfg,
            init.session.control_established,
            init.session.last_control_rx_ms,
            init.reconnect_grace_until_ms,
            now_ms,
        ));

        if init.session.state == SnifferState::ReconCapturing {
            let mut ctx = init.session.as_ctx();
            if sniffer_recon::run_recon_tick(
                &mut init.wifi,
                &init.promisc_rx,
                &mut ctx,
                &init.rx_rx,
                now_ms,
                init.sniffer_id,
                &mut recon_cap,
                &mut init.recon,
            )? {
                continue;
            }
        }

        if init.session.state != SnifferState::Scanning {
            if init.session.pending_ack && init.session.control_established {
                if let Some(mac) = init.session.control_mac {
                    let control_ch = init.session.schedule.control_channel;
                    if try_set_channel_with_recovery(&mut init.wifi, control_ch, "idle heartbeat") {
                        flush_pending_time_set_standalone(
                            &mut init.session.pending_time_set,
                            &mut init.session.last_time_set_seq,
                            init.session.last_rtt_time_set_ms,
                        );
                        let mut ctx = init.session.as_ctx();
                        let _ = sniffer_dispatch::run_peer_refresh_join(
                            init.sniffer_id,
                            control_ch,
                            || {
                                let _ = ctx.drain_rx(&init.rx_rx);
                            },
                        );
                        if init.session.control_established {
                            if let Err(e) = send_ack(
                                mac,
                                init.sniffer_id,
                                init.session.last_cmd_seq,
                                init.session.state,
                                &init.session.cfg,
                            ) {
                                wardriver_stats::SNIFFER_ACK_TX_FAILED
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                warn!("sniffer: idle send_ack failed (non-fatal): {e:#}");
                            } else {
                                init.session.pending_ack = false;
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        // TX power is capped once at boot (see `wifi_tx_power::apply_default_cap_once`).

        if init.session.cfg.role == SnifferRole::Ble as u8 {
            let mut ctx = init.session.as_ctx();
            if sniffer_ble::run_ble_scan_tick(
                &mut init.wifi,
                &init.rx_rx,
                &mut init.seq,
                &mut init.scan_batch_id,
                &mut init.outbound_bulk,
                &mut ctx,
                &mut init.last_on_control_ms,
                &mut init.schedule_sig_prev,
            )? {
                continue;
            }
        }

        flush_pending_time_set_standalone(
            &mut init.session.pending_time_set,
            &mut init.session.last_time_set_seq,
            init.session.last_rtt_time_set_ms,
        );

        let mut ctx = init.session.as_ctx();

        let control_ch = ctx.schedule.control_channel;
        let Some(hop) = run_wifi_sweep_hop(
            &mut init.wifi,
            &init.promisc_rx,
            &mut ctx,
            &init.rx_rx,
            init.last_on_control_ms,
            &mut init.next_channel_idx,
            &mut init.schedule_sig_prev,
            init.seq,
        )?
        else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };

        run_wifi_sweep_finish(
            &mut init.wifi,
            &mut ctx,
            &init.rx_rx,
            control_ch,
            hop,
            &mut init.outbound_bulk,
            &mut init.last_on_control_ms,
            &mut init.next_channel_idx,
            &mut init.schedule_sig_prev,
            &mut init.seq,
            &mut init.scan_batch_id,
        )?;

        if *ctx.state != SnifferState::Scanning {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Sniffer shutdown phase.
///
/// The original `run_sniffer` is an infinite loop with no post-loop statements, so this phase
/// is currently a no-op. It exists to complete the init / main-loop / shutdown structure
/// (mirroring the control firmware's split) so any future teardown code has a home.
fn sniffer_shutdown(_init: &mut SnifferInit<'_>) {}
