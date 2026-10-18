//! PackLink v1 — transport-agnostic frame codec between wolfPACK gateway and phone.
//!
//! Primary transport is gateway SoftAP with plaintext TCP on [`PACKLINK_TCP_PORT`].

mod channel_plan_sync;

use anyhow::{anyhow, Result};
use wardriver_proto::{CHANNELS_MAX, PROTO_VERSION};

pub use channel_plan_sync::*;

/// Max sniffers in phone-owned fleet roster (`PolicySyncV1::roster`).
pub const ROSTER_MAX: usize = 48;

/// Max SSID watch entries in `PolicySyncV1` detection tail.
pub const SSID_WATCH_MAX: usize = 16;

/// Per-row wire size for [`SnifferStatusV1`] (fixed layout).
pub const SNIFFER_STATUS_WIRE_LEN: usize = 22;

/// `SnifferStatus` batch payload prefix: `gateway_mono_ms` u32 before row count.
pub const SNIFFER_STATUS_BATCH_HEADER_LEN: usize = 4;

/// Per-entry size on wire: `sniffer_id` u32 + `mac` [6] + `fleet_slot` u8.
const ROSTER_ENTRY_WIRE_LEN: usize = 4 + 6 + 1 + 1;

/// PackLink wire version (independent of wolfPACK [`PROTO_VERSION`]).
pub const PACKLINK_VERSION: u8 = 1;

/// Frame marker — avoids parsing random TCP bytes as PackLink.
pub const MAGIC: [u8; 4] = *b"PLNK";

/// TCP port for PackLink on the gateway SoftAP (single client).
pub const PACKLINK_TCP_PORT: u16 = 4710;

/// Max UTF-8 length for a phone UUID in [`PhoneHelloV1`].
pub const PHONE_ID_MAX_LEN: usize = 36;

/// Max SoftAP SSID octets in [`SoftApConfigV1`] (802.11 SSID field).
pub const SOFTAP_SSID_MAX_LEN: usize = 32;

/// Min WPA2 passphrase length in [`SoftApConfigV1`].
pub const SOFTAP_PASS_MIN_LEN: usize = 8;

/// Max WPA2 passphrase length in [`SoftApConfigV1`].
pub const SOFTAP_PASS_MAX_LEN: usize = 63;

const HEADER_LEN: usize = 14;
const CRC_LEN: usize = 4;

/// PackLink message type (payload layout per type).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgType {
    GatewayInfo = 1,
    PolicySync = 2,
    ControlCmd = 3,
    EventNotify = 4,
    ObservationBulk = 5,
    SpoolReplay = 6,
    TimeSync = 7,
    SnifferStatus = 8,
    /// Phone → gateway: start a fleet RF capture job (write `control_cmd` characteristic).
    CaptureCmd = 9,
    /// Gateway → phone: capture progress snapshot (notify `event_notify`).
    CaptureStatus = 10,
    /// Phone → gateway: stable phone identifier for TOFU pinning (plaintext TCP).
    PhoneHello = 11,
    /// Gateway → phone: TOFU accepted; session may proceed.
    TrustOk = 12,
    /// Gateway → phone: TOFU rejected (wrong phone or already pinned).
    TrustReject = 13,
    /// Phone → gateway: new SoftAP SSID + password (trusted phone only).
    SoftApConfig = 14,
    /// Phone → gateway: clear pinned phone trust.
    TrustForget = 15,
    /// Gateway → phone: PolicySync applied (or rejected).
    PolicySyncAck = 16,
    /// Phone → gateway: operator channel plan (control-parity).
    ChannelPlanSync = 17,
    /// Gateway → phone: ChannelPlanSync applied (or rejected).
    ChannelPlanSyncAck = 18,
    /// Gateway → phone: physical BOOT unbind cleared gateway RAM roster.
    FleetUnbindNotify = 19,
    /// Gateway → phone: batch of probe request observations.
    ProbeObservationBulk = 20,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::GatewayInfo),
            2 => Some(Self::PolicySync),
            3 => Some(Self::ControlCmd),
            4 => Some(Self::EventNotify),
            5 => Some(Self::ObservationBulk),
            6 => Some(Self::SpoolReplay),
            7 => Some(Self::TimeSync),
            8 => Some(Self::SnifferStatus),
            9 => Some(Self::CaptureCmd),
            10 => Some(Self::CaptureStatus),
            11 => Some(Self::PhoneHello),
            12 => Some(Self::TrustOk),
            13 => Some(Self::TrustReject),
            14 => Some(Self::SoftApConfig),
            15 => Some(Self::TrustForget),
            16 => Some(Self::PolicySyncAck),
            17 => Some(Self::ChannelPlanSync),
            18 => Some(Self::ChannelPlanSyncAck),
            19 => Some(Self::FleetUnbindNotify),
            20 => Some(Self::ProbeObservationBulk),
            _ => None,
        }
    }
}

/// Frame flags.
pub mod flags {
    pub const FRAGMENT: u8 = 1 << 0;
    pub const FRAGMENT_END: u8 = 1 << 1;
    pub const PRIORITY_ALERT: u8 = 1 << 2;
    pub const SPOOL_REPLAY: u8 = 1 << 3;
}

/// Parsed PackLink frame header + payload slice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    pub version: u8,
    pub msg_type: MsgType,
    pub flags: u8,
    pub seq: u32,
    pub payload: &'a [u8],
}

/// Gateway → phone status snapshot (read on `gateway_info` or notify).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayInfoV1 {
    pub gateway_mac: [u8; 6],
    pub firmware_version: u16,
    pub packlink_version: u8,
    pub proto_version: u8,
    pub spool_bytes_total: u32,
    pub spool_bytes_used: u32,
    pub peer_slots_free: u8,
    pub peer_join_waiters: u8,
    pub fleet_id: u32,
    pub gateway_boot_id: u32,
    /// Optional capability tail (`gateway_capabilities::*`); `0` on legacy payloads.
    pub capabilities: u8,
}

/// One adopted sniffer in the phone-owned fleet roster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterEntryV1 {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub fleet_slot: u8,
    /// wolfPACK sniffer role wire value (`0` WiFi, `1` BLE).
    pub role: u8,
}

/// Phone → gateway fleet policy (write `policy_sync`).
#[derive(Clone, Debug, PartialEq)]
pub struct PolicySyncV1 {
    pub scan_allowed: bool,
    pub wardrive_suppressed: bool,
    pub geofence_enabled: bool,
    pub geofence_lat_min: f64,
    pub geofence_lat_max: f64,
    pub geofence_lon_min: f64,
    pub geofence_lon_max: f64,
    pub denylist_bssids: Vec<[u8; 6]>,
    pub denylist_ssids: Vec<String>,
    /// Phone-owned fleet roster (replaces gateway NVS roster).
    pub roster: Vec<RosterEntryV1>,
    /// Channel sweep schedule (same tail layout as wolfPACK `ControlCmdV1`).
    pub dwell_ms: u16,
    pub chan_count: u8,
    pub channels: [u8; CHANNELS_MAX],
    /// Per-method WiFi detection disable bitmask (wolfPACK `detection_disable_wifi_mask`).
    pub detection_disable_wifi_mask: u32,
    /// Per-method BLE detection disable bitmask (wolfPACK `detection_disable_ble_mask`).
    pub detection_disable_ble_mask: u32,
    /// Operator SSID watchlist (beacon match → alert).
    pub ssid_watch: Vec<String>,
    /// Per-method Remote ID disable mask (Wi-Fi `64..=95`, BLE `96..=127`).
    pub remote_id_disable_mask: u32,
    /// BLE-role sniffers use active LE scan (SCAN_REQ) when true; passive when false (default).
    pub ble_active_scan: bool,
}

/// Compact per-sniffer row for SoftAP notify batching (`SnifferStatus` frames).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnifferStatusV1 {
    pub sniffer_id: u32,
    pub mac: [u8; 6],
    pub last_seen_ms: u32,
    pub state: u8,
    pub role: u8,
    pub cpu_load_pct: u8,
    pub heap_avail_pct: u8,
    pub fleet_slot: u8,
    pub reboot_count: u16,
    /// Matches control/`SnifferSlot::adoption_state` (`ADOPTION_*`).
    pub adoption_state: u8,
    /// Present when batch carries plan-extension tail.
    pub plan_ext: Option<SnifferStatusPlanExtV1>,
    /// Present when batch carries gateway adoption-retry tail.
    pub adopt_ext: Option<SnifferStatusAdoptExtV1>,
}

/// Gateway → phone ack for an uplink [`PolicySync`](MsgType::PolicySync).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicySyncAckV1 {
    /// Frame `seq` from the phone's uplink `PolicySync`.
    pub policy_seq: u32,
    /// `0` = applied OK, `1` = rejected / apply failed.
    pub result: u8,
    /// `0` = none, `1` = parse/unpack failure, `2` = apply failed.
    pub error_code: u8,
}

pub mod policy_sync_ack_result {
    pub const OK: u8 = 0;
    pub const REJECTED: u8 = 1;
}

pub mod policy_sync_ack_error {
    pub const NONE: u8 = 0;
    pub const PARSE: u8 = 1;
    pub const APPLY: u8 = 2;
}

/// `SnifferSlot::adoption_state` / PackLink `adoption_state` values.
pub mod adoption_state {
    pub const EMPTY: u8 = 0;
    pub const PENDING: u8 = 1;
    pub const ADOPTED: u8 = 2;
    pub const REMOVED: u8 = 3;
}

/// True when RAM policy mirror holds wardrive roster or scan intent.
#[must_use]
pub fn policy_has_fleet_config(policy: &PolicySyncV1) -> bool {
    !policy.roster.is_empty() || policy.scan_allowed
}

/// True when `policy.roster` contains the `(sniffer_id, mac)` pair.
#[must_use]
pub fn roster_allows(policy: &PolicySyncV1, sniffer_id: u32, mac: &[u8; 6]) -> bool {
    policy
        .roster
        .iter()
        .any(|e| e.sniffer_id == sniffer_id && e.mac == *mac)
}

/// Roster role wire value for `mac`, or `0` (WiFi) when not rostered.
#[must_use]
pub fn roster_role_for_mac(policy: &PolicySyncV1, mac: &[u8; 6]) -> u8 {
    policy
        .roster
        .iter()
        .find(|e| e.mac == *mac)
        .map(|e| e.role.min(1))
        .unwrap_or(0)
}

/// Realtime alert / geofence / fleet event (notify `event_notify`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventNotifyV1 {
    pub event_kind: u8,
    pub sniffer_id: u32,
    pub unix_ms: u64,
    pub payload_len: u16,
    pub payload: Vec<u8>,
}

pub mod event_kind {
    pub const DETECTION_WIFI: u8 = 1;
    pub const DETECTION_BLE: u8 = 2;
    pub const SSID_WATCH: u8 = 3;
    pub const GEOFENCE_LEAVE: u8 = 4;
    pub const SPOOL_WATERMARK: u8 = 5;
    pub const GATEWAY_RESET: u8 = 6;
    pub const REMOTE_ID: u8 = 7;
}

/// Observation bulk header; trailing bytes are opaque wolfPACK reassembled batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservationBulkV1 {
    pub batch_id: u32,
    pub sniffer_id: u32,
    pub scan_end_unix_ms: u64,
    pub wifi_ap_count: u16,
    pub ble_count: u16,
    pub payload: Vec<u8>,
}

/// Probe observation bulk header; trailing bytes are concatenated ProbeObservationWire entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeObservationBulkV1 {
    pub batch_id: u32,
    pub sniffer_id: u32,
    pub scan_end_unix_ms: u64,
    pub scan_end_ms: u32,
    pub count: u16,
    pub payload: Vec<u8>,
}

/// Spool replay chunk header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpoolReplayV1 {
    pub spool_offset: u32,
    pub chunk_total: u32,
    pub chunk_index: u32,
    pub payload: Vec<u8>,
}

/// Phone → gateway capture job request (`CaptureCmd` on `control_cmd` write).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureCmdV1 {
    /// Job selector (`capture_job_id::WIFI_PROBE_PCAP` = 1).
    pub job_id: u8,
    /// Capture window length in seconds (gateway clamps to its max).
    pub duration_sec: u16,
}

/// Gateway → phone capture progress (`CaptureStatus` notify).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureStatusV1 {
    /// Non-zero while a capture session is running.
    pub active: u8,
    /// Non-zero while fleet scan is held for capture (mirrors wolfPACK `recon_hold`).
    pub hold: u8,
    /// Milliseconds until capture end (`stats::now_ms` semantics); 0 when idle.
    pub remaining_ms: u32,
    /// ESP-NOW `ReconWifiFrag` payloads accepted this session.
    pub frags_rx: u32,
    /// Raw bytes appended to gateway capture buffer (not necessarily valid pcapng yet).
    pub bytes_written: u32,
    /// `capture_error::NONE` when ok; see [`capture_error`].
    pub error_code: u8,
}

/// Capture job identifiers (`CaptureCmdV1::job_id`).
pub mod capture_job_id {
    /// 60 s WiFi probe request capture (control node: pcapng on SD; gateway MVP: RAM stub).
    pub const WIFI_PROBE_PCAP: u8 = 1;
}

/// Phone → gateway TOFU hello (plaintext, first frame on TCP connect).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneHelloV1 {
    /// Stable phone identifier (typically a UUID string).
    pub phone_id: String,
}

/// Gateway → phone TOFU accept (plaintext).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrustOkV1;

/// Gateway → phone TOFU reject (plaintext).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrustRejectV1 {
    pub reason: u8,
}

/// Phone → gateway SoftAP credential update (trusted phone only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SoftApConfigV1 {
    pub ssid: String,
    pub password: String,
}

/// `TrustRejectV1::reason` values.
pub mod trust_reject_reason {
    pub const PHONE_MISMATCH: u8 = 1;
    pub const ALREADY_PINNED: u8 = 2;
}

/// `CaptureStatusV1::error_code` values.
pub mod capture_error {
    pub const NONE: u8 = 0;
    pub const BUSY: u8 = 1;
    pub const NO_SNIFFERS: u8 = 2;
    pub const BUFFER_FULL: u8 = 3;
    pub const UNSUPPORTED_JOB: u8 = 4;
}

/// Bidirectional monotonic / wall-clock sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSyncV1 {
    pub phone_unix_ms: u64,
    pub phone_mono_ms: u32,
    pub gateway_mono_ms: u32,
    pub time_set_seq: u32,
}

fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}

/// Pack a frame with CRC32 trailing.
pub fn pack_frame(msg_type: MsgType, flags: u8, seq: u32, payload: &[u8]) -> Vec<u8> {
    let payload_len = payload.len();
    assert!(payload_len <= u16::MAX as usize);
    let mut out = Vec::with_capacity(HEADER_LEN + payload_len + CRC_LEN);
    out.extend_from_slice(&MAGIC);
    out.push(PACKLINK_VERSION);
    out.push(msg_type as u8);
    out.push(flags);
    out.push(0);
    out.extend_from_slice(&(payload_len as u16).to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(payload);
    let c = crc32(&out);
    out.extend_from_slice(&c.to_le_bytes());
    out
}

/// Unpack and verify CRC; returns header fields and payload slice.
pub fn unpack_frame(wire: &[u8]) -> Result<Frame<'_>> {
    if wire.len() < HEADER_LEN + CRC_LEN {
        return Err(anyhow!("packlink frame too short"));
    }
    if wire[0..4] != MAGIC {
        return Err(anyhow!("packlink bad magic"));
    }
    let version = wire[4];
    if version != PACKLINK_VERSION {
        return Err(anyhow!("packlink version {version} != {PACKLINK_VERSION}"));
    }
    let msg_type = MsgType::from_u8(wire[5]).ok_or_else(|| anyhow!("unknown msg_type"))?;
    let flags = wire[6];
    let payload_len = u16::from_le_bytes([wire[8], wire[9]]) as usize;
    let seq = u32::from_le_bytes([wire[10], wire[11], wire[12], wire[13]]);
    let payload_end = HEADER_LEN + payload_len;
    if wire.len() < payload_end + CRC_LEN {
        return Err(anyhow!("packlink truncated payload"));
    }
    let payload = &wire[HEADER_LEN..payload_end];
    let expect_crc = u32::from_le_bytes([
        wire[payload_end],
        wire[payload_end + 1],
        wire[payload_end + 2],
        wire[payload_end + 3],
    ]);
    let got_crc = crc32(&wire[..payload_end]);
    if expect_crc != got_crc {
        return Err(anyhow!("packlink crc mismatch"));
    }
    Ok(Frame {
        version,
        msg_type,
        flags,
        seq,
        payload,
    })
}

pub fn pack_gateway_info(info: &GatewayInfoV1) -> Vec<u8> {
    let mut p = Vec::with_capacity(32);
    p.extend_from_slice(&info.gateway_mac);
    p.extend_from_slice(&info.firmware_version.to_le_bytes());
    p.push(info.packlink_version);
    p.push(info.proto_version);
    p.extend_from_slice(&info.spool_bytes_total.to_le_bytes());
    p.extend_from_slice(&info.spool_bytes_used.to_le_bytes());
    p.push(info.peer_slots_free);
    p.push(info.peer_join_waiters);
    p.extend_from_slice(&info.fleet_id.to_le_bytes());
    p.extend_from_slice(&info.gateway_boot_id.to_le_bytes());
    p.push(info.capabilities);
    pack_frame(MsgType::GatewayInfo, 0, 0, &p)
}

/// Pack `PolicySync` with a caller-chosen frame `seq` (phone correlates [`PolicySyncAckV1::policy_seq`]).
pub fn pack_policy_sync_seq(policy: &PolicySyncV1, seq: u32) -> Vec<u8> {
    let p = policy_sync_payload_bytes(policy);
    pack_frame(MsgType::PolicySync, 0, seq, &p)
}

/// Inner `PolicySync` payload bytes (no PLNK frame header); suitable for NVS blob storage.
#[must_use]
pub fn policy_sync_payload_bytes(policy: &PolicySyncV1) -> Vec<u8> {
    let mut p = Vec::new();
    p.push(u8::from(policy.scan_allowed));
    p.push(u8::from(policy.wardrive_suppressed));
    p.push(u8::from(policy.geofence_enabled));
    p.extend_from_slice(&policy.geofence_lat_min.to_le_bytes());
    p.extend_from_slice(&policy.geofence_lat_max.to_le_bytes());
    p.extend_from_slice(&policy.geofence_lon_min.to_le_bytes());
    p.extend_from_slice(&policy.geofence_lon_max.to_le_bytes());
    p.push(policy.denylist_bssids.len().min(255) as u8);
    p.push(policy.denylist_ssids.len().min(255) as u8);
    for bssid in policy.denylist_bssids.iter().take(255) {
        p.extend_from_slice(bssid);
    }
    for ssid in policy.denylist_ssids.iter().take(255) {
        let bytes = ssid.as_bytes();
        let n = bytes.len().min(32);
        p.push(n as u8);
        p.extend_from_slice(&bytes[..n]);
    }
    let roster_n = policy.roster.len().min(ROSTER_MAX);
    p.push(roster_n as u8);
    for entry in policy.roster.iter().take(ROSTER_MAX) {
        p.extend_from_slice(&entry.sniffer_id.to_le_bytes());
        p.extend_from_slice(&entry.mac);
        p.push(entry.fleet_slot);
        p.push(entry.role.min(1));
    }
    p.extend_from_slice(&policy.dwell_ms.to_le_bytes());
    p.push(policy.chan_count.min(CHANNELS_MAX as u8));
    p.extend_from_slice(&policy.channels);
    p.extend_from_slice(&policy.detection_disable_wifi_mask.to_le_bytes());
    p.extend_from_slice(&policy.detection_disable_ble_mask.to_le_bytes());
    let watch_n = policy.ssid_watch.len().min(SSID_WATCH_MAX);
    p.push(watch_n as u8);
    for ssid in policy.ssid_watch.iter().take(SSID_WATCH_MAX) {
        let bytes = ssid.as_bytes();
        let n = bytes.len().min(32);
        p.push(n as u8);
        p.extend_from_slice(&bytes[..n]);
    }
    p.extend_from_slice(&policy.remote_id_disable_mask.to_le_bytes());
    p.push(u8::from(policy.ble_active_scan));
    p
}

pub fn unpack_policy_sync(payload: &[u8]) -> Result<PolicySyncV1> {
    if payload.len() < 35 {
        return Err(anyhow!("policy_sync payload short"));
    }
    let scan_allowed = payload[0] != 0;
    let wardrive_suppressed = payload[1] != 0;
    let geofence_enabled = payload[2] != 0;
    let geofence_lat_min = f64::from_le_bytes(payload[3..11].try_into()?);
    let geofence_lat_max = f64::from_le_bytes(payload[11..19].try_into()?);
    let geofence_lon_min = f64::from_le_bytes(payload[19..27].try_into()?);
    let geofence_lon_max = f64::from_le_bytes(payload[27..35].try_into()?);
    let bssid_count = payload[35] as usize;
    let ssid_count = payload[36] as usize;
    let mut off = 37;
    let mut denylist_bssids = Vec::with_capacity(bssid_count);
    for _ in 0..bssid_count {
        if off + 6 > payload.len() {
            return Err(anyhow!("policy_sync bssid truncated"));
        }
        let mut bssid = [0u8; 6];
        bssid.copy_from_slice(&payload[off..off + 6]);
        denylist_bssids.push(bssid);
        off += 6;
    }
    let mut denylist_ssids = Vec::with_capacity(ssid_count);
    for _ in 0..ssid_count {
        if off >= payload.len() {
            return Err(anyhow!("policy_sync ssid truncated"));
        }
        let n = payload[off] as usize;
        off += 1;
        if off + n > payload.len() {
            return Err(anyhow!("policy_sync ssid bytes truncated"));
        }
        denylist_ssids.push(
            std::str::from_utf8(&payload[off..off + n])
                .map_err(|_| anyhow!("policy_sync ssid utf8"))?
                .to_string(),
        );
        off += n;
    }
    // Roster + channel schedule are optional on legacy payloads (end after denylists).
    let (roster, dwell_ms, chan_count, channels, off_after_schedule) = if off >= payload.len() {
        (Vec::new(), 0, 0, [0u8; CHANNELS_MAX], off)
    } else {
        if off + 1 > payload.len() {
            return Err(anyhow!("policy_sync roster_count truncated"));
        }
        let roster_count = payload[off] as usize;
        off += 1;
        if roster_count > ROSTER_MAX {
            return Err(anyhow!("policy_sync roster_count > {ROSTER_MAX}"));
        }
        let mut roster = Vec::with_capacity(roster_count);
        for _ in 0..roster_count {
            if off + ROSTER_ENTRY_WIRE_LEN > payload.len() {
                return Err(anyhow!("policy_sync roster entry truncated"));
            }
            let sniffer_id = u32::from_le_bytes(payload[off..off + 4].try_into()?);
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&payload[off + 4..off + 10]);
            let fleet_slot = payload[off + 10];
            let role = payload[off + 11].min(1);
            roster.push(RosterEntryV1 {
                sniffer_id,
                mac,
                fleet_slot,
                role,
            });
            off += ROSTER_ENTRY_WIRE_LEN;
        }
        // dwell_ms u16 + chan_count u8 + channels[CHANNELS_MAX]
        let schedule_len = 2 + 1 + CHANNELS_MAX;
        if off + schedule_len > payload.len() {
            return Err(anyhow!("policy_sync channel schedule truncated"));
        }
        let dwell_ms = u16::from_le_bytes([payload[off], payload[off + 1]]);
        off += 2;
        let chan_count = payload[off].min(CHANNELS_MAX as u8);
        off += 1;
        let mut channels = [0u8; CHANNELS_MAX];
        channels.copy_from_slice(&payload[off..off + CHANNELS_MAX]);
        off += CHANNELS_MAX;
        (roster, dwell_ms, chan_count, channels, off)
    };
    // Detection tail (detection masks + SSID watch) — omitted on pre-detection payloads.
    let mut detection_disable_wifi_mask = 0u32;
    let mut detection_disable_ble_mask = 0u32;
    let mut ssid_watch = Vec::new();
    let mut det_off = off_after_schedule;
    if det_off + 8 <= payload.len() {
        detection_disable_wifi_mask = u32::from_le_bytes(payload[det_off..det_off + 4].try_into()?);
        det_off += 4;
        detection_disable_ble_mask = u32::from_le_bytes(payload[det_off..det_off + 4].try_into()?);
        det_off += 4;
        if det_off < payload.len() {
            let watch_count = payload[det_off] as usize;
            det_off += 1;
            if watch_count > SSID_WATCH_MAX {
                return Err(anyhow!("policy_sync ssid_watch_count > {SSID_WATCH_MAX}"));
            }
            for _ in 0..watch_count {
                if det_off >= payload.len() {
                    return Err(anyhow!("policy_sync ssid_watch truncated"));
                }
                let n = payload[det_off] as usize;
                det_off += 1;
                if det_off + n > payload.len() {
                    return Err(anyhow!("policy_sync ssid_watch bytes truncated"));
                }
                ssid_watch.push(
                    std::str::from_utf8(&payload[det_off..det_off + n])
                        .map_err(|_| anyhow!("policy_sync ssid_watch utf8"))?
                        .to_string(),
                );
                det_off += n;
            }
        }
    }
    let remote_id_disable_mask = if det_off + 4 <= payload.len() {
        u32::from_le_bytes(payload[det_off..det_off + 4].try_into()?)
    } else {
        0
    };
    // ble_active_scan: trailing u8 after remote_id_disable_mask (default 0 = passive).
    let ble_active_scan = if det_off + 5 <= payload.len() {
        payload[det_off + 4] != 0
    } else {
        false
    };
    Ok(PolicySyncV1 {
        scan_allowed,
        wardrive_suppressed,
        geofence_enabled,
        geofence_lat_min,
        geofence_lat_max,
        geofence_lon_min,
        geofence_lon_max,
        denylist_bssids,
        denylist_ssids,
        roster,
        dwell_ms,
        chan_count,
        channels,
        detection_disable_wifi_mask,
        detection_disable_ble_mask,
        ssid_watch,
        remote_id_disable_mask,
        ble_active_scan,
    })
}

fn pack_sniffer_status_payload(gateway_mono_ms: u32, rows: &[SnifferStatusV1]) -> Vec<u8> {
    let n = rows.len().min(255);
    let mut p =
        Vec::with_capacity(SNIFFER_STATUS_BATCH_HEADER_LEN + 1 + n * SNIFFER_STATUS_WIRE_LEN);
    p.extend_from_slice(&gateway_mono_ms.to_le_bytes());
    p.push(n as u8);
    for row in rows.iter().take(255) {
        p.extend_from_slice(&row.sniffer_id.to_le_bytes());
        p.extend_from_slice(&row.mac);
        p.extend_from_slice(&row.last_seen_ms.to_le_bytes());
        p.push(row.state);
        p.push(row.role);
        p.push(row.cpu_load_pct);
        p.push(row.heap_avail_pct);
        p.push(row.fleet_slot);
        p.extend_from_slice(&row.reboot_count.to_le_bytes());
        p.push(row.adoption_state);
    }
    let has_plan_ext = rows.iter().take(255).any(|r| r.plan_ext.is_some());
    if has_plan_ext {
        p.push(SNIFFER_STATUS_EXT_VERSION);
        for row in rows.iter().take(255) {
            let ext = row.plan_ext.unwrap_or_default();
            p.extend_from_slice(&pack_sniffer_status_plan_ext(&ext));
        }
    }
    let has_adopt_ext = rows.iter().take(255).any(|r| r.adopt_ext.is_some());
    if has_adopt_ext {
        p.push(SNIFFER_STATUS_ADOPT_EXT_VERSION);
        for row in rows.iter().take(255) {
            let ext = row.adopt_ext.unwrap_or_default();
            p.extend_from_slice(&pack_sniffer_status_adopt_ext(&ext));
        }
    }
    p
}

/// Pack one sniffer row as a `SnifferStatus` frame (batch count = 1).
pub fn pack_sniffer_status(row: &SnifferStatusV1, gateway_mono_ms: u32, seq: u32) -> Vec<u8> {
    pack_sniffer_status_batch(std::slice::from_ref(row), gateway_mono_ms, seq)
}

/// Pack multiple sniffer rows in one `SnifferStatus` frame payload.
pub fn pack_sniffer_status_batch(
    rows: &[SnifferStatusV1],
    gateway_mono_ms: u32,
    seq: u32,
) -> Vec<u8> {
    let payload = pack_sniffer_status_payload(gateway_mono_ms, rows);
    pack_frame(MsgType::SnifferStatus, 0, seq, &payload)
}

fn unpack_sniffer_status_rows(payload: &[u8]) -> Result<(u32, Vec<SnifferStatusV1>)> {
    if payload.len() < SNIFFER_STATUS_BATCH_HEADER_LEN + 1 {
        return Err(anyhow!("sniffer_status payload short"));
    }
    let gateway_mono_ms = u32::from_le_bytes(payload[0..4].try_into()?);
    let count = payload[4] as usize;
    let mut off = 5usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if off + SNIFFER_STATUS_WIRE_LEN > payload.len() {
            return Err(anyhow!("sniffer_status row truncated"));
        }
        let sniffer_id = u32::from_le_bytes(payload[off..off + 4].try_into()?);
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&payload[off + 4..off + 10]);
        let last_seen_ms = u32::from_le_bytes(payload[off + 10..off + 14].try_into()?);
        let state = payload[off + 14];
        let role = payload[off + 15];
        let cpu_load_pct = payload[off + 16];
        let heap_avail_pct = payload[off + 17];
        let fleet_slot = payload[off + 18];
        let reboot_count = u16::from_le_bytes([payload[off + 19], payload[off + 20]]);
        let adoption_state = payload[off + 21];
        out.push(SnifferStatusV1 {
            sniffer_id,
            mac,
            last_seen_ms,
            state,
            role,
            cpu_load_pct,
            heap_avail_pct,
            fleet_slot,
            reboot_count,
            adoption_state,
            plan_ext: None,
            adopt_ext: None,
        });
        off += SNIFFER_STATUS_WIRE_LEN;
    }
    if off < payload.len() && payload[off] == SNIFFER_STATUS_EXT_VERSION {
        off += 1;
        for row in out.iter_mut() {
            if off + SNIFFER_STATUS_EXT_WIRE_LEN > payload.len() {
                return Err(anyhow!("sniffer_status ext truncated"));
            }
            row.plan_ext = Some(unpack_sniffer_status_plan_ext(
                &payload[off..off + SNIFFER_STATUS_EXT_WIRE_LEN],
            )?);
            off += SNIFFER_STATUS_EXT_WIRE_LEN;
        }
    }
    if off < payload.len() && payload[off] == SNIFFER_STATUS_ADOPT_EXT_VERSION {
        off += 1;
        for row in out.iter_mut() {
            if off + SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN > payload.len() {
                return Err(anyhow!("sniffer_status adopt ext truncated"));
            }
            row.adopt_ext = Some(unpack_sniffer_status_adopt_ext(
                &payload[off..off + SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN],
            )?);
            off += SNIFFER_STATUS_ADOPT_EXT_WIRE_LEN;
        }
    }
    Ok((gateway_mono_ms, out))
}

/// Unpack the first row from a `SnifferStatus` payload (batch count must be 1).
pub fn unpack_sniffer_status(payload: &[u8]) -> Result<(u32, SnifferStatusV1)> {
    let (gateway_mono_ms, rows) = unpack_sniffer_status_rows(payload)?;
    let row = rows
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("sniffer_status batch empty"))?;
    Ok((gateway_mono_ms, row))
}

/// Unpack all rows from a `SnifferStatus` batch payload.
pub fn unpack_sniffer_status_batch(payload: &[u8]) -> Result<(u32, Vec<SnifferStatusV1>)> {
    unpack_sniffer_status_rows(payload)
}

/// Pack gateway → phone `PolicySyncAck`.
pub fn pack_policy_sync_ack(ack: &PolicySyncAckV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(6);
    p.extend_from_slice(&ack.policy_seq.to_le_bytes());
    p.push(ack.result);
    p.push(ack.error_code);
    pack_frame(MsgType::PolicySyncAck, 0, seq, &p)
}

/// Unpack `PolicySyncAck` payload.
pub fn unpack_policy_sync_ack(payload: &[u8]) -> Result<PolicySyncAckV1> {
    if payload.len() < 6 {
        return Err(anyhow!("policy_sync_ack payload short"));
    }
    Ok(PolicySyncAckV1 {
        policy_seq: u32::from_le_bytes(payload[0..4].try_into()?),
        result: payload[4],
        error_code: payload[5],
    })
}

pub fn pack_event_notify(ev: &EventNotifyV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(15 + ev.payload.len());
    p.push(ev.event_kind);
    p.extend_from_slice(&ev.sniffer_id.to_le_bytes());
    p.extend_from_slice(&ev.unix_ms.to_le_bytes());
    p.extend_from_slice(&ev.payload_len.to_le_bytes());
    p.extend_from_slice(&ev.payload);
    let flags = if event_notify_is_priority(ev.event_kind) {
        flags::PRIORITY_ALERT
    } else {
        0
    };
    pack_frame(MsgType::EventNotify, flags, seq, &p)
}

fn event_notify_is_priority(kind: u8) -> bool {
    matches!(
        kind,
        event_kind::DETECTION_WIFI
            | event_kind::DETECTION_BLE
            | event_kind::SSID_WATCH
            | event_kind::REMOTE_ID
    )
}

pub fn unpack_event_notify(payload: &[u8]) -> Result<EventNotifyV1> {
    if payload.len() < 15 {
        return Err(anyhow!("event_notify header short"));
    }
    let event_kind = payload[0];
    let sniffer_id = u32::from_le_bytes(payload[1..5].try_into()?);
    let unix_ms = u64::from_le_bytes(payload[5..13].try_into()?);
    let payload_len = u16::from_le_bytes([payload[13], payload[14]]) as usize;
    if payload.len() < 15 + payload_len {
        return Err(anyhow!("event_notify payload truncated"));
    }
    Ok(EventNotifyV1 {
        event_kind,
        sniffer_id,
        unix_ms,
        payload_len: payload_len as u16,
        payload: payload[15..15 + payload_len].to_vec(),
    })
}

pub fn pack_observation_bulk(bulk: &ObservationBulkV1, seq: u32, frag_flags: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(20 + bulk.payload.len());
    p.extend_from_slice(&bulk.batch_id.to_le_bytes());
    p.extend_from_slice(&bulk.sniffer_id.to_le_bytes());
    p.extend_from_slice(&bulk.scan_end_unix_ms.to_le_bytes());
    p.extend_from_slice(&bulk.wifi_ap_count.to_le_bytes());
    p.extend_from_slice(&bulk.ble_count.to_le_bytes());
    p.extend_from_slice(&bulk.payload);
    pack_frame(MsgType::ObservationBulk, frag_flags, seq, &p)
}

pub fn unpack_observation_bulk(payload: &[u8]) -> Result<ObservationBulkV1> {
    if payload.len() < 20 {
        return Err(anyhow!("observation_bulk header short"));
    }
    let batch_id = u32::from_le_bytes(payload[0..4].try_into()?);
    let sniffer_id = u32::from_le_bytes(payload[4..8].try_into()?);
    let scan_end_unix_ms = u64::from_le_bytes(payload[8..16].try_into()?);
    let wifi_ap_count = u16::from_le_bytes(payload[16..18].try_into()?);
    let ble_count = u16::from_le_bytes(payload[18..20].try_into()?);
    Ok(ObservationBulkV1 {
        batch_id,
        sniffer_id,
        scan_end_unix_ms,
        wifi_ap_count,
        ble_count,
        payload: payload[20..].to_vec(),
    })
}

pub fn pack_probe_observation_bulk(
    bulk: &ProbeObservationBulkV1,
    seq: u32,
    frag_flags: u8,
) -> Vec<u8> {
    let mut p = Vec::with_capacity(22 + bulk.payload.len());
    p.extend_from_slice(&bulk.batch_id.to_le_bytes());
    p.extend_from_slice(&bulk.sniffer_id.to_le_bytes());
    p.extend_from_slice(&bulk.scan_end_unix_ms.to_le_bytes());
    p.extend_from_slice(&bulk.scan_end_ms.to_le_bytes());
    p.extend_from_slice(&bulk.count.to_le_bytes());
    p.extend_from_slice(&bulk.payload);
    pack_frame(MsgType::ProbeObservationBulk, frag_flags, seq, &p)
}

pub fn unpack_probe_observation_bulk(payload: &[u8]) -> Result<ProbeObservationBulkV1> {
    if payload.len() < 22 {
        return Err(anyhow!("probe_observation_bulk header short"));
    }
    let batch_id = u32::from_le_bytes(payload[0..4].try_into()?);
    let sniffer_id = u32::from_le_bytes(payload[4..8].try_into()?);
    let scan_end_unix_ms = u64::from_le_bytes(payload[8..16].try_into()?);
    let scan_end_ms = u32::from_le_bytes(payload[16..20].try_into()?);
    let count = u16::from_le_bytes(payload[20..22].try_into()?);
    let expected_payload_len = count as usize * wardriver_proto::PROBE_OBSERVATION_WIRE_SIZE;
    if payload.len() < 22 + expected_payload_len {
        return Err(anyhow!(
            "probe_observation_bulk payload too short: expected {} got {}",
            22 + expected_payload_len,
            payload.len()
        ));
    }
    Ok(ProbeObservationBulkV1 {
        batch_id,
        sniffer_id,
        scan_end_unix_ms,
        scan_end_ms,
        count,
        payload: payload[22..22 + expected_payload_len].to_vec(),
    })
}

pub fn pack_spool_replay(chunk: &SpoolReplayV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + chunk.payload.len());
    p.extend_from_slice(&chunk.spool_offset.to_le_bytes());
    p.extend_from_slice(&chunk.chunk_total.to_le_bytes());
    p.extend_from_slice(&chunk.chunk_index.to_le_bytes());
    p.extend_from_slice(&chunk.payload);
    pack_frame(MsgType::SpoolReplay, flags::SPOOL_REPLAY, seq, &p)
}

pub fn unpack_spool_replay(payload: &[u8]) -> Result<SpoolReplayV1> {
    if payload.len() < 12 {
        return Err(anyhow!("spool_replay header short"));
    }
    Ok(SpoolReplayV1 {
        spool_offset: u32::from_le_bytes(payload[0..4].try_into()?),
        chunk_total: u32::from_le_bytes(payload[4..8].try_into()?),
        chunk_index: u32::from_le_bytes(payload[8..12].try_into()?),
        payload: payload[12..].to_vec(),
    })
}

pub fn pack_time_sync(ts: &TimeSyncV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(20);
    p.extend_from_slice(&ts.phone_unix_ms.to_le_bytes());
    p.extend_from_slice(&ts.phone_mono_ms.to_le_bytes());
    p.extend_from_slice(&ts.gateway_mono_ms.to_le_bytes());
    p.extend_from_slice(&ts.time_set_seq.to_le_bytes());
    pack_frame(MsgType::TimeSync, 0, seq, &p)
}

pub fn unpack_time_sync(payload: &[u8]) -> Result<TimeSyncV1> {
    if payload.len() < 20 {
        return Err(anyhow!("time_sync payload short"));
    }
    Ok(TimeSyncV1 {
        phone_unix_ms: u64::from_le_bytes(payload[0..8].try_into()?),
        phone_mono_ms: u32::from_le_bytes(payload[8..12].try_into()?),
        gateway_mono_ms: u32::from_le_bytes(payload[12..16].try_into()?),
        time_set_seq: u32::from_le_bytes(payload[16..20].try_into()?),
    })
}

pub fn pack_capture_cmd(cmd: &CaptureCmdV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(3);
    p.push(cmd.job_id);
    p.extend_from_slice(&cmd.duration_sec.to_le_bytes());
    pack_frame(MsgType::CaptureCmd, 0, seq, &p)
}

pub fn unpack_capture_cmd(payload: &[u8]) -> Result<CaptureCmdV1> {
    if payload.len() < 3 {
        return Err(anyhow!("capture_cmd payload short"));
    }
    Ok(CaptureCmdV1 {
        job_id: payload[0],
        duration_sec: u16::from_le_bytes([payload[1], payload[2]]),
    })
}

pub fn pack_capture_status(status: &CaptureStatusV1, seq: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(15);
    p.push(status.active);
    p.push(status.hold);
    p.extend_from_slice(&status.remaining_ms.to_le_bytes());
    p.extend_from_slice(&status.frags_rx.to_le_bytes());
    p.extend_from_slice(&status.bytes_written.to_le_bytes());
    p.push(status.error_code);
    pack_frame(MsgType::CaptureStatus, 0, seq, &p)
}

pub fn unpack_capture_status(payload: &[u8]) -> Result<CaptureStatusV1> {
    if payload.len() < 15 {
        return Err(anyhow!("capture_status payload short"));
    }
    Ok(CaptureStatusV1 {
        active: payload[0],
        hold: payload[1],
        remaining_ms: u32::from_le_bytes(payload[2..6].try_into()?),
        frags_rx: u32::from_le_bytes(payload[6..10].try_into()?),
        bytes_written: u32::from_le_bytes(payload[10..14].try_into()?),
        error_code: payload[14],
    })
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

fn validate_softap_ssid(ssid: &str) -> Result<()> {
    if ssid.is_empty() || ssid.len() > SOFTAP_SSID_MAX_LEN {
        return Err(anyhow!("softap ssid length invalid"));
    }
    Ok(())
}

fn validate_softap_password(pass: &str) -> Result<()> {
    let n = pass.len();
    if n < SOFTAP_PASS_MIN_LEN || n > SOFTAP_PASS_MAX_LEN {
        return Err(anyhow!("softap password length invalid"));
    }
    Ok(())
}

/// Pack phone → gateway `PhoneHello` (plaintext TCP).
pub fn pack_phone_hello(hello: &PhoneHelloV1, seq: u32) -> Vec<u8> {
    validate_phone_id(&hello.phone_id).expect("pack_phone_hello: invalid phone_id");
    let bytes = hello.phone_id.as_bytes();
    let mut p = Vec::with_capacity(1 + bytes.len());
    p.push(bytes.len() as u8);
    p.extend_from_slice(bytes);
    pack_frame(MsgType::PhoneHello, 0, seq, &p)
}

/// Unpack `PhoneHello` payload bytes.
pub fn unpack_phone_hello(payload: &[u8]) -> Result<PhoneHelloV1> {
    if payload.is_empty() {
        return Err(anyhow!("phone_hello payload empty"));
    }
    let n = payload[0] as usize;
    if n == 0 || n > PHONE_ID_MAX_LEN || payload.len() < 1 + n {
        return Err(anyhow!("phone_hello id truncated"));
    }
    let phone_id = std::str::from_utf8(&payload[1..1 + n])
        .map_err(|_| anyhow!("phone_hello id utf8"))?
        .to_string();
    validate_phone_id(&phone_id)?;
    Ok(PhoneHelloV1 { phone_id })
}

/// Pack gateway → phone `TrustOk` (empty payload).
pub fn pack_trust_ok(seq: u32) -> Vec<u8> {
    pack_frame(MsgType::TrustOk, 0, seq, &[])
}

/// Pack gateway → phone `TrustReject`.
pub fn pack_trust_reject(reject: &TrustRejectV1, seq: u32) -> Vec<u8> {
    pack_frame(MsgType::TrustReject, 0, seq, &[reject.reason])
}

/// Pack phone → gateway `SoftApConfig`.
pub fn pack_softap_config(cfg: &SoftApConfigV1, seq: u32) -> Vec<u8> {
    validate_softap_ssid(&cfg.ssid).expect("pack_softap_config: invalid ssid");
    validate_softap_password(&cfg.password).expect("pack_softap_config: invalid password");
    let ssid_bytes = cfg.ssid.as_bytes();
    let pass_bytes = cfg.password.as_bytes();
    let mut p = Vec::with_capacity(2 + ssid_bytes.len() + pass_bytes.len());
    p.push(ssid_bytes.len() as u8);
    p.extend_from_slice(ssid_bytes);
    p.push(pass_bytes.len() as u8);
    p.extend_from_slice(pass_bytes);
    pack_frame(MsgType::SoftApConfig, 0, seq, &p)
}

/// Unpack `SoftApConfig` payload bytes.
pub fn unpack_softap_config(payload: &[u8]) -> Result<SoftApConfigV1> {
    if payload.len() < 2 {
        return Err(anyhow!("softap_config payload short"));
    }
    let ssid_len = payload[0] as usize;
    if ssid_len == 0 || ssid_len > SOFTAP_SSID_MAX_LEN || payload.len() < 1 + ssid_len + 1 {
        return Err(anyhow!("softap_config ssid truncated"));
    }
    let ssid = std::str::from_utf8(&payload[1..1 + ssid_len])
        .map_err(|_| anyhow!("softap_config ssid utf8"))?
        .to_string();
    let pass_off = 1 + ssid_len;
    let pass_len = payload[pass_off] as usize;
    if pass_len < SOFTAP_PASS_MIN_LEN
        || pass_len > SOFTAP_PASS_MAX_LEN
        || payload.len() < pass_off + 1 + pass_len
    {
        return Err(anyhow!("softap_config password truncated"));
    }
    let password = std::str::from_utf8(&payload[pass_off + 1..pass_off + 1 + pass_len])
        .map_err(|_| anyhow!("softap_config password utf8"))?
        .to_string();
    validate_softap_ssid(&ssid)?;
    validate_softap_password(&password)?;
    Ok(SoftApConfigV1 { ssid, password })
}

/// Pack gateway → phone `FleetUnbindNotify` (empty payload).
pub fn pack_fleet_unbind_notify(seq: u32) -> Vec<u8> {
    pack_frame(MsgType::FleetUnbindNotify, 0, seq, &[])
}

/// Default gateway info for tests and bench.
#[must_use]
pub fn default_gateway_info() -> GatewayInfoV1 {
    GatewayInfoV1 {
        gateway_mac: [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
        firmware_version: 1,
        packlink_version: PACKLINK_VERSION,
        proto_version: PROTO_VERSION,
        spool_bytes_total: 256 * 1024,
        spool_bytes_used: 0,
        peer_slots_free: 16,
        peer_join_waiters: 0,
        fleet_id: 0x3257_4452,
        gateway_boot_id: 1,
        capabilities: 0,
    }
}
