//! NVS-backed PackLink TOFU trust: one pinned phone per gateway.

use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::info;
use wardriver_packlink::PHONE_ID_MAX_LEN;

/// NVS namespace for phone trust and optional SoftAP credential overrides.
const NS: &str = "packlink_trust";

const K_PHONE_ID: &str = "phone_id";
const K_TRUSTED: &str = "trusted";
const K_SOFTAP_SSID: &str = "ap_ssid";
const K_SOFTAP_PASS: &str = "ap_pass";

static NVS: OnceLock<EspNvsPartition<NvsDefault>> = OnceLock::new();
static PENDING_SOFTAP: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();

/// Bind the default NVS partition (call once from gateway boot).
pub fn bind_nvs(partition: EspNvsPartition<NvsDefault>) {
    let _ = NVS.set(partition);
    let _ = PENDING_SOFTAP.get_or_init(|| Mutex::new(None));
}

fn nvs_partition() -> Option<&'static EspNvsPartition<NvsDefault>> {
    NVS.get()
}

fn open_nvs() -> Result<EspNvs<NvsDefault>> {
    let part = nvs_partition().ok_or_else(|| anyhow!("packlink_trust: NVS not bound"))?;
    EspNvs::new(part.clone(), NS, true).context("open packlink_trust NVS")
}

fn validate_phone_id(phone_id: &str) -> Result<()> {
    if phone_id.is_empty() || phone_id.len() > PHONE_ID_MAX_LEN {
        return Err(anyhow!("phone_id length invalid"));
    }
    if !phone_id.is_ascii() {
        return Err(anyhow!("phone_id must be ASCII"));
    }
    Ok(())
}

/// Return true when a phone id is pinned in NVS.
#[must_use]
pub fn is_trusted() -> bool {
    open_nvs()
        .ok()
        .and_then(|nvs| nvs.get_u8(K_TRUSTED).ok().flatten())
        .map(|v| v != 0)
        .unwrap_or(false)
}

/// Load the pinned phone id when [`is_trusted`] is true.
pub fn trusted_phone_id() -> Result<String> {
    if !is_trusted() {
        return Err(anyhow!("no trusted phone"));
    }
    let nvs = open_nvs()?;
    let mut buf = [0u8; PHONE_ID_MAX_LEN + 1];
    let slice = nvs
        .get_str(K_PHONE_ID, &mut buf)?
        .ok_or_else(|| anyhow!("phone_id missing"))?;
    validate_phone_id(slice)?;
    Ok(slice.to_string())
}

/// Outcome of a TOFU `PhoneHello` check (NVS on main loop only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifyPhoneHelloOutcome {
    Accepted,
    PhoneMismatch,
}

/// TOFU: pin the first phone id, or accept a reconnect from the same id.
///
/// Call only from the gateway main loop (or [`crate::packlink_nvs_defer`] drain).
pub(crate) fn verify_phone_hello_outcome(
    phone_id: &str,
) -> Result<VerifyPhoneHelloOutcome, String> {
    validate_phone_id(phone_id).map_err(|e| e.to_string())?;
    if !is_trusted() {
        let nvs = open_nvs().map_err(|e| e.to_string())?;
        nvs.set_str(K_PHONE_ID, phone_id)
            .map_err(|e| format!("nvs set phone_id: {e:?}"))?;
        nvs.set_u8(K_TRUSTED, 1)
            .map_err(|e| format!("nvs set trusted: {e:?}"))?;
        info!("packlink_trust: pinned phone_id={phone_id}");
        return Ok(VerifyPhoneHelloOutcome::Accepted);
    }
    let pinned = trusted_phone_id().map_err(|e| e.to_string())?;
    if pinned == phone_id {
        Ok(VerifyPhoneHelloOutcome::Accepted)
    } else {
        Ok(VerifyPhoneHelloOutcome::PhoneMismatch)
    }
}

/// Clear pinned phone trust (`TrustForget` or operator reset).
pub fn clear_trust() -> Result<()> {
    let nvs = open_nvs()?;
    nvs.remove(K_PHONE_ID)
        .map_err(|e| anyhow!("nvs remove phone_id: {e:?}"))?;
    nvs.set_u8(K_TRUSTED, 0)
        .map_err(|e| anyhow!("nvs clear trusted: {e:?}"))?;
    info!("packlink_trust: trust cleared");
    Ok(())
}

/// Load SoftAP credentials: NVS override, else fleet build-time defaults.
pub fn load_softap_creds() -> Result<(String, String)> {
    let nvs = open_nvs()?;
    let mut ssid_buf = [0u8; 33];
    let mut pass_buf = [0u8; 64];
    let ssid = if let Some(s) = nvs.get_str(K_SOFTAP_SSID, &mut ssid_buf)? {
        if !s.is_empty() {
            s.to_string()
        } else {
            wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_SSID.to_string()
        }
    } else {
        wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_SSID.to_string()
    };
    let pass = if let Some(p) = nvs.get_str(K_SOFTAP_PASS, &mut pass_buf)? {
        if !p.is_empty() {
            p.to_string()
        } else {
            wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_PASS.to_string()
        }
    } else {
        wardriver_shared::fleet_mgmt::FLEET_MGMT_AP_PASS.to_string()
    };
    Ok((ssid, pass))
}

/// Persist SoftAP credentials and queue an AP restart from the main loop.
pub fn queue_softap_config(ssid: String, password: String) -> Result<()> {
    let nvs = open_nvs()?;
    nvs.set_str(K_SOFTAP_SSID, ssid.as_str())
        .map_err(|e| anyhow!("nvs set ap_ssid: {e:?}"))?;
    nvs.set_str(K_SOFTAP_PASS, password.as_str())
        .map_err(|e| anyhow!("nvs set ap_pass: {e:?}"))?;
    if let Ok(mut g) = PENDING_SOFTAP.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some((ssid, password));
    }
    info!("packlink_trust: queued SoftAP credential update");
    Ok(())
}

/// Take a pending SoftAP restart request, if any.
pub fn take_pending_softap_restart() -> Option<(String, String)> {
    PENDING_SOFTAP.get()?.lock().ok().and_then(|mut g| g.take())
}
