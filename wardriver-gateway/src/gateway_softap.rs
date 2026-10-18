//! Gateway management SoftAP (APSTA + ESP-NOW on [`CONTROL_AP_CHANNEL`]).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{anyhow, Result};
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::wifi::{
    AccessPointConfiguration, AuthMethod, BlockingWifi, Configuration, EspWifi,
};
use log::{info, warn};
use wardriver_fleet_realtime::CONTROL_AP_CHANNEL;
use wardriver_shared::task_wdt;
use wardriver_stats as stats;
use wardriver_stats::fleet::{
    CONTROL_SOFTAP_GATEWAY, CONTROL_SOFTAP_NETMASK, CONTROL_SOFTAP_SUBNET, SOFTAP_MAX_CONNECTIONS,
};

use crate::packlink_trust;

/// Wi-Fi assoc without PackLink TCP must survive join + Android `requestNetwork` + TCP connect.
const SOFTAP_STALE_ASSOC_MS: u64 = 45_000;

static SOFTAP_ASSOC_WITHOUT_TCP_MS: AtomicU32 = AtomicU32::new(0);

const DHCP_POOL_START_OCTET: u8 = 2;
const DHCP_POOL_END_OCTET: u8 = 254;

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

fn prepare_softap_before_config() -> Result<()> {
    wardriver_shared::wifi_ap_tune::prepare_softap_before_config(false)
}

fn configure_ap_protocols() {
    let result = wardriver_shared::wifi_ap_tune::configure_ap_protocols();
    if result.set_protocols_rc != 0 {
        warn!(
            "gateway_softap: esp_wifi_set_protocols(AP) rc={}",
            result.set_protocols_rc
        );
    }
    if result.disable_11b_rc != 0 {
        warn!(
            "gateway_softap: esp_wifi_config_11b_rate(AP, disable) rc={}",
            result.disable_11b_rc
        );
    }
    if result.bandwidth_rc != 0 {
        warn!(
            "gateway_softap: esp_wifi_config_bandwidth(AP, HT20) rc={}",
            result.bandwidth_rc
        );
    }
}

fn ap_configuration(ssid: &str, pass: &str) -> Result<AccessPointConfiguration> {
    Ok(AccessPointConfiguration {
        ssid: ssid
            .try_into()
            .map_err(|_| anyhow!("gateway SoftAP SSID too long"))?,
        ssid_hidden: false,
        channel: CONTROL_AP_CHANNEL,
        auth_method: AuthMethod::WPA2Personal,
        password: pass
            .try_into()
            .map_err(|_| anyhow!("gateway SoftAP password too long"))?,
        max_connections: SOFTAP_MAX_CONNECTIONS,
        ..Default::default()
    })
}

fn wait_ap_netif_ready(wifi: &BlockingWifi<EspWifi<'static>>, timeout_ms: u64) -> Result<()> {
    let deadline = stats::now_ms().saturating_add(timeout_ms);
    while stats::now_ms() < deadline {
        let up = wifi
            .wifi()
            .ap_netif()
            .is_up()
            .map_err(|e| anyhow!("ap_netif.is_up: {e:?}"))?;
        if up {
            return Ok(());
        }
        task_wdt::sleep_feeding_wdt(Duration::from_millis(50));
    }
    Err(anyhow!("timed out waiting for SoftAP netif"))
}

fn configure_management_softap_netif(wifi: &BlockingWifi<EspWifi<'static>>) -> Result<()> {
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
            warn!("gateway_softap: dhcps_stop rc={rc}");
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
            warn!("gateway_softap: dhcps lease pool rc={rc}");
        }
        let rc = esp_idf_sys::esp_netif_dhcps_start(netif);
        if rc != esp_idf_sys::ESP_OK {
            warn!("gateway_softap: dhcps_start rc={rc}");
        }
    }

    info!(
        "gateway_softap: {} DHCP .{}.-.{} (phone PackLink; sniffers ESP-NOW only)",
        fmt_ipv4(CONTROL_SOFTAP_GATEWAY),
        DHCP_POOL_START_OCTET,
        DHCP_POOL_END_OCTET,
    );
    Ok(())
}

/// Start APSTA with management SoftAP on the fleet channel.
pub fn init_apsta(wifi: &mut BlockingWifi<EspWifi<'static>>) -> Result<()> {
    let (ssid, pass) = packlink_trust::load_softap_creds()?;
    apply_softap_config(wifi, &ssid, &pass)
}

/// Re-apply SoftAP SSID/password (e.g. after phone `SoftApConfig`).
pub fn apply_softap_config(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    ssid: &str,
    pass: &str,
) -> Result<()> {
    prepare_softap_before_config()?;
    let ap = ap_configuration(ssid, pass)?;
    let mixed = Configuration::Mixed(Default::default(), ap);
    wifi.set_configuration(&mixed)?;
    wifi.start()?;
    configure_ap_protocols();
    info!(
        "gateway_softap: management AP ssid={ssid} ch={CONTROL_AP_CHANNEL} max_conn={SOFTAP_MAX_CONNECTIONS}"
    );
    wait_ap_netif_ready(wifi, 20_000)?;
    configure_management_softap_netif(wifi)?;
    Ok(())
}

/// Apply a pending SoftAP restart from NVS queue, if requested.
pub fn apply_pending_softap_restart(wifi: &mut BlockingWifi<EspWifi<'static>>) {
    if let Some((ssid, pass)) = packlink_trust::take_pending_softap_restart() {
        if let Err(e) = apply_softap_config(wifi, &ssid, &pass) {
            warn!("gateway_softap: restart after credential change failed: {e:#}");
        }
    }
}

/// SoftAP associated station count (Wi-Fi layer, not PackLink TCP).
pub fn softap_station_count() -> i32 {
    let mut list: esp_idf_sys::wifi_sta_list_t = unsafe { std::mem::zeroed() };
    let rc = unsafe { esp_idf_sys::esp_wifi_ap_get_sta_list(&mut list) };
    if rc != esp_idf_sys::ESP_OK as i32 {
        return 0;
    }
    list.num
}

/// Deauth all SoftAP stations so the single phone slot can reconnect.
pub fn deauth_all_softap_stations() {
    let mut list: esp_idf_sys::wifi_sta_list_t = unsafe { std::mem::zeroed() };
    let rc = unsafe { esp_idf_sys::esp_wifi_ap_get_sta_list(&mut list) };
    if rc != esp_idf_sys::ESP_OK as i32 || list.num <= 0 {
        return;
    }
    for i in 0..list.num {
        let sta = &list.sta[i as usize];
        let mut aid: u16 = 0;
        let aid_rc =
            unsafe { esp_idf_sys::esp_wifi_ap_get_sta_aid(sta.mac.as_ptr(), &mut aid as *mut u16) };
        if aid_rc == esp_idf_sys::ESP_OK as i32 && aid != 0 {
            let _ = unsafe { esp_idf_sys::esp_wifi_deauth_sta(aid) };
        }
    }
}

/// Evict Wi-Fi associations only after a grace window without PackLink TCP.
///
/// Immediate deauth on every main-loop tick breaks phone join: Wi-Fi comes up before TCP and
/// `phone_connected()` is still false. Session drop still deauths promptly when TCP ends.
pub fn evict_stale_softap_clients(phone_connected: bool) {
    if phone_connected {
        SOFTAP_ASSOC_WITHOUT_TCP_MS.store(0, Ordering::Relaxed);
        return;
    }
    let count = softap_station_count();
    if count <= 0 {
        SOFTAP_ASSOC_WITHOUT_TCP_MS.store(0, Ordering::Relaxed);
        return;
    }
    let now = stats::now_ms();
    let since = SOFTAP_ASSOC_WITHOUT_TCP_MS.load(Ordering::Relaxed);
    if since == 0 {
        SOFTAP_ASSOC_WITHOUT_TCP_MS.store(now.min(u64::from(u32::MAX)) as u32, Ordering::Relaxed);
        return;
    }
    if now.saturating_sub(u64::from(since)) < SOFTAP_STALE_ASSOC_MS {
        return;
    }
    info!(
        "gateway_softap: evicting stale assoc (stations={count}, no PackLink TCP for {} ms)",
        now.saturating_sub(u64::from(since)),
    );
    deauth_all_softap_stations();
    SOFTAP_ASSOC_WITHOUT_TCP_MS.store(0, Ordering::Relaxed);
}

/// SoftAP interface MAC (beacon BSSID).
pub fn read_softap_mac() -> Result<[u8; 6]> {
    let mut mac = [0u8; 6];
    let rc = unsafe {
        esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr(),
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_SOFTAP,
        )
    };
    if rc != 0 {
        return Err(anyhow!("esp_read_mac(SOFTAP) rc={rc}"));
    }
    Ok(mac)
}
