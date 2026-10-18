//! Helpers shared with the control boot path: SoftAP netif wait and form parsing for `/setup`.
//!
//! First-time WiFi configuration uses the same HTTP server as the dashboard (`web.rs`):
//! `GET /setup` and `POST /save`. The device always boots the control stack so `/` is the
//! fleet dashboard even before a home WiFi SSID is saved.

use std::collections::BTreeMap;
use std::time::Duration;

use wardriver_shared::task_wdt;

use anyhow::{anyhow, Result};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

/// Wait until the SoftAP LwIP interface reports up.
///
/// Do **not** use [`BlockingWifi::wait_netif_up`] for AP bring-up when the WiFi
/// driver is in [`esp_idf_svc::wifi::Configuration::Mixed`]: `EspWifi::is_up()` short-circuits on
/// `is_connected()`, which stays false until the STA associates, so the wait
/// always times out even though the AP is already beaconing. Polling the AP
/// netif avoids that footgun; AP-only mode works with this path as well.
pub(crate) fn wait_ap_netif_ready(
    wifi: &BlockingWifi<EspWifi<'static>>,
    timeout: Duration,
) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
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
    Err(anyhow!(
        "timed out after {timeout:?} waiting for SoftAP netif; check WiFi / sdkconfig"
    ))
}

fn parse_form_urlencoded(body: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in body.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some(kv) => kv,
            None => (pair, ""),
        };
        let k = url_decode(k);
        let v = url_decode(v);
        out.insert(k, v);
    }
    out
}

// Exposed for `web.rs` reuse (same parsing rules).
pub fn parse_form_urlencoded_for_web(body: &str) -> BTreeMap<String, String> {
    parse_form_urlencoded(body)
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h1 = from_hex(bytes[i + 1]);
                let h2 = from_hex(bytes[i + 2]);
                if let (Some(a), Some(b)) = (h1, h2) {
                    out.push((a << 4) | b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
