//! NVS persistence for gateway RAM `PolicySyncV1` mirror (survives reboot).

use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use esp_idf_svc::nvs::{EspNvs, EspNvsPartition, NvsDefault};
use log::warn;
use wardriver_packlink::{policy_sync_payload_bytes, unpack_policy_sync, PolicySyncV1};

const NS: &str = "gateway_policy";
const K_POLICY_V1: &str = "policy_v1";
const MAX_BLOB: usize = 16_384;

static NVS: OnceLock<EspNvsPartition<NvsDefault>> = OnceLock::new();
static PENDING_SAVE: Mutex<Option<Vec<u8>>> = Mutex::new(None);

pub fn bind_nvs(partition: EspNvsPartition<NvsDefault>) {
    let _ = NVS.set(partition);
}

fn open_nvs() -> Result<EspNvs<NvsDefault>> {
    let part = NVS
        .get()
        .ok_or_else(|| anyhow::anyhow!("gateway_policy: NVS not bound"))?;
    EspNvs::new(part.clone(), NS, true).context("open gateway_policy NVS")
}

/// Stage policy blob for main-loop NVS write (safe from PackLink pthread).
pub fn stage_save(policy: &PolicySyncV1) {
    let blob = policy_sync_payload_bytes(policy);
    if let Ok(mut pending) = PENDING_SAVE.lock() {
        *pending = Some(blob);
    }
}

/// Drain staged policy write on the gateway main loop.
pub fn drain_save() {
    let blob = PENDING_SAVE.lock().ok().and_then(|mut g| g.take());
    let Some(blob) = blob else {
        return;
    };
    if let Err(e) = write_blob(&blob) {
        warn!("gateway_policy: save: {e:#}");
    }
}

fn write_blob(blob: &[u8]) -> Result<()> {
    if blob.len() > MAX_BLOB {
        return Err(anyhow::anyhow!("policy blob too large: {}", blob.len()));
    }
    let nvs = open_nvs()?;
    nvs.set_blob(K_POLICY_V1, blob)
        .map_err(|e| anyhow::anyhow!("nvs set_blob policy_v1: {e:?}"))?;
    Ok(())
}

/// Load persisted policy payload from NVS (`None` when unset or corrupt).
pub fn load_payload() -> Option<Vec<u8>> {
    let nvs = open_nvs().ok()?;
    let mut raw = vec![0u8; MAX_BLOB];
    let slice = nvs.get_blob(K_POLICY_V1, &mut raw).ok()??;
    Some(slice.to_vec())
}

/// Load and unpack policy from NVS.
pub fn load_policy() -> Option<PolicySyncV1> {
    let payload = load_payload()?;
    unpack_policy_sync(&payload).ok()
}
