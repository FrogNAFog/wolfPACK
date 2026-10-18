//! Remote ID detection with per-source cooldown and UAS snapshot alerts.

use std::collections::HashMap;

use wardriver_proto::BleEntryV1;

use crate::detection::{
    is_remote_id_ble_method_disabled, is_remote_id_wifi_method_disabled, DetectionSignalKind,
    RemoteIdBleDetectionMethod, RemoteIdWifiDetectionMethod,
};
use crate::remote_id::ble_extract::extract_ble_remote_id_payload;
use crate::remote_id::decode::decode_open_drone_id_payload;
use crate::remote_id::types::RemoteIdUasData;
use crate::remote_id::wifi_extract::extract_wifi_remote_id_from_mgmt;

/// Per-source alert cooldown (30 s between alerts for the same source MAC).
pub const PER_SRC_COOLDOWN_MS: u64 = 30_000;

const SRC_IDLE_EVICT_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Default)]
struct SrcState {
    last_seen_ms: u64,
    last_alert_ms: u64,
}

pub struct RemoteIdDetector {
    per_src: HashMap<[u8; 6], SrcState>,
}

impl RemoteIdDetector {
    pub fn new() -> Self {
        Self {
            per_src: HashMap::new(),
        }
    }

    pub fn prune(&mut self, now_ms: u64) {
        self.per_src
            .retain(|_, s| now_ms.saturating_sub(s.last_seen_ms) <= SRC_IDLE_EVICT_MS);
    }

    pub fn ingest_ble(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        entry: &BleEntryV1,
        adv: &[u8],
        disable_ble_mask: u32,
    ) -> Option<RemoteIdAlert> {
        let payload = extract_ble_remote_id_payload(adv)?;
        let method = if adv_windows_service_data_fffa(adv) {
            RemoteIdBleDetectionMethod::ServiceDataFffa
        } else {
            RemoteIdBleDetectionMethod::LegacyMfg0002
        };
        if is_remote_id_ble_method_disabled(disable_ble_mask, method.as_u8()) {
            return None;
        }
        self.ingest_payload(
            sniffer_id,
            now_ms,
            entry.addr,
            entry.rssi,
            0,
            DetectionSignalKind::Ble,
            method.as_u8(),
            payload,
        )
    }

    pub fn ingest_wifi(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        mpdu: &[u8],
        src: [u8; 6],
        rssi: i8,
        channel: u8,
        disable_wifi_mask: u32,
    ) -> Option<RemoteIdAlert> {
        let (payload, transport) = extract_wifi_remote_id_from_mgmt(mpdu)?;
        let method = match transport {
            crate::remote_id::types::RemoteIdWifiTransport::Beacon => {
                RemoteIdWifiDetectionMethod::BeaconVendorIe
            }
            crate::remote_id::types::RemoteIdWifiTransport::Nan => {
                RemoteIdWifiDetectionMethod::NanAction
            }
        };
        if is_remote_id_wifi_method_disabled(disable_wifi_mask, method.as_u8()) {
            return None;
        }
        self.ingest_payload(
            sniffer_id,
            now_ms,
            src,
            rssi,
            channel,
            DetectionSignalKind::Wifi,
            method.as_u8(),
            payload,
        )
    }

    fn ingest_payload(
        &mut self,
        sniffer_id: u32,
        now_ms: u64,
        src: [u8; 6],
        rssi: i8,
        channel: u8,
        signal_kind: DetectionSignalKind,
        method: u8,
        payload: &[u8],
    ) -> Option<RemoteIdAlert> {
        let st = self.per_src.entry(src).or_default();
        st.last_seen_ms = now_ms;
        if st.last_alert_ms != 0 && now_ms.saturating_sub(st.last_alert_ms) < PER_SRC_COOLDOWN_MS {
            return None;
        }

        let mut uas_data = RemoteIdUasData::default();
        if decode_open_drone_id_payload(payload, &mut uas_data).is_none() {
            return None;
        }
        if !uas_data.is_alert_ready() {
            return None;
        }

        st.last_alert_ms = now_ms;
        Some(RemoteIdAlert {
            sniffer_id,
            src,
            rssi,
            channel,
            method,
            signal_kind,
            uas_data,
        })
    }
}

impl Default for RemoteIdDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteIdAlert {
    pub sniffer_id: u32,
    pub src: [u8; 6],
    pub rssi: i8,
    pub channel: u8,
    pub method: u8,
    pub signal_kind: DetectionSignalKind,
    pub uas_data: RemoteIdUasData,
}

fn adv_windows_service_data_fffa(adv: &[u8]) -> bool {
    let mut i = 0usize;
    while i < adv.len() {
        let len = adv[i] as usize;
        i += 1;
        if len == 0 {
            break;
        }
        if i + len > adv.len() {
            break;
        }
        if adv[i] == 0x16 && len >= 3 && adv.get(i + 1..i + 3) == Some(&[0xFA, 0xFF][..]) {
            return true;
        }
        i += len;
    }
    false
}
