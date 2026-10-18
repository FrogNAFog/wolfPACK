//! ESP-NOW wire formats and shared protocol types (greenfield v1).
//!
//! Manual pack/unpack; frames stay within the ESP-NOW v2 payload budget (≤ [`ESP_NOW_MAX_FRAME_LEN`]).
//! Fleet control and sniffers must share the same [`PROTO_VERSION`].

mod scan_entry_tlv;
pub use scan_entry_tlv::{
    pack_scan_entry_tlv, parse_ble_entry_tlv, parse_scan_entry_tlv, parse_wifi_ap_entry_tlv,
    scan_entry_tlv_wire_len, ScanEntryExtras, SCAN_TLV_TAG_BLE_COD, SCAN_TLV_TAG_RCOI_IE,
};

use anyhow::{anyhow, Result};

/// ESP-NOW / fleet wire format version (bump only with coordinated fleet flash).
pub const PROTO_VERSION: u8 = 8;

/// Maximum scan channels per sniffer schedule (wire `channels` array length). Sized for
/// worst-case full 5 GHz + DFS hop (28 channels) with headroom.
pub const CHANNELS_MAX: usize = 32;

/// Management SoftAP RF channel and ESP-NOW command/return channel (2.4 GHz default).
#[cfg(not(feature = "mgmt-ap-5ghz"))]
pub const CONTROL_AP_CHANNEL: u8 = 11;

/// 5 GHz UNII-3 non-DFS management channel (opt-in `mgmt-ap-5ghz` feature).
#[cfg(feature = "mgmt-ap-5ghz")]
pub const CONTROL_AP_CHANNEL: u8 = 161;

// --- Wire contract (control vs sniffer); keep in sync with `control/run.rs` + `sniffer.rs` ---
//
// FrameType::ControlCmd — control → sniffer (broadcast + per-MAC unicast). Sniffer accepts
// broadcast or unicast; learns/rebinds `control_mac` from `frame.src`. Session: `control_boot_id`,
// `cmd_seq`. Commands: Start/Stop scanning, SetChannelSchedule, Ping, SetRole.
// PROTO v7 (wolfPACK): `Ping` uses a short wire form (no dwell/channels); see `pack_ping_vec`.
// Full ControlCmd body still carries `desired_scanning` on Start/Stop/SetChannelSchedule/SetRole.
// Broadcast `Ping` populates `peer_slots_free` + `peer_join_waiters` join hints.
//
// FrameType::PeerJoinNack — control → broadcast (PROTO v6 / wolfPACK); cap / queue backpressure.
//
// FrameType::SnifferAck — sniffer → control after handling a command (incl. Ping heartbeat).
//
// FrameType::ScanFrag — sniffer → control; fragmented by (sniffer_id, scan_batch_id, seq, frag_idx).
// PROTO v6 (wolfPACK): `flags` after common header; [`FRAG_FLAG_TRANSPORT_END`] on one frame
// per control-channel transport burst (one dump visit), not per reasm group.
// PROTO v6 (wolfPACK): AP entries use variable-length SSID on the wire (14 + ssid_len bytes).
//
// FrameType::BssidDenylist — control → sniffer (unicast + broadcast); WiGLE privacy merged list.
//
// FrameType::SsidDenylist — control → sniffer (unicast + broadcast); manual WiGLE SSID exclusions.
//
// FrameType::SsidWatchlist — control → sniffer; operator SSID watchlist (beacon match → alert).
//
// FrameType::SsidWatchAlert — sniffer → control; immediate beacon SSID watch hit (ESP-NOW only).
//
// FrameType::RemoteIdAlert — sniffer → control; OpenDroneID / ASTM F3411 UAS detection (ESP-NOW only).
//
// FrameType::TimeSyncPing / TimeSyncPong — optional monotonic time sync (see pack/unpack).
//
// FrameType::BleScanFrag — sniffer → control; fragmented BLE batch (same `flags` as ScanFrag).
//
// FrameType::ProbeReqFrag — sniffer → control; wildcard probe-burst telemetry (not detection alerts;
// same `flags` as ScanFrag).
//
// FrameType::ProbeObservationFrag — sniffer → control; non-wildcard probe observations carrying
// SSIDs for hidden network discovery (fragmented; same `flags` as ScanFrag).
//
// FrameType::DetectionAlert — sniffer → control; LPR-camera WiFi detection (ESP-NOW only, immediate).
//
// FrameType::DetectionBleAlert — sniffer → control; LPR-camera BLE detection (ESP-NOW only, immediate).
//
// FrameType::TimeSet — control → sniffer (unicast); GPS-derived Unix ms when wall clock trusted.
//
// FrameType::ReconCmd / ReconWifiFrag — recon capture job + raw MPDU frags (sniffer ↔ control).
//
// FrameType::SnifferHello — sniffer → broadcast; fast join. Control answers with ControlWelcome
// (unicast) plus immediate Start/Stop, denylist, optional TimeSet (see run.rs).
//
// FrameType::ControlWelcome — control → sniffer (unicast); bootstrap schedule + boot id.
//
// FrameType::PeerJoin — sniffer → broadcast (PROTO v6 / wolfPACK); roster-adopted dump visit.
// Unencrypted; precedes encrypted bulk frags. [`PeerJoinReason`] has no Hello variant.
//
// FrameType::PeerJoinAck — control → sniffer (unicast); after `ensure_peer`. Carries
// [`PeerJoinAckV1::fleet_slot`] and [`PeerJoinAckV1::peer_epoch`] for encrypted session scope.
//
// FrameType::PeerJoinNack — control → broadcast (PROTO v6 wolfPACK); cap / queue backpressure.
// Unencrypted; targeted by [`PeerJoinNackV1::sniffer_id`]. Sniffer leaves control channel and
// retries `PeerJoin` after `retry_after_ms` unless outbound bulk is near capacity (fail-safe).
//
// Pairing: sniffers may use NVS `control_mac` as hint; `control_established` gates TX until a
// valid control frame is received. Periodic `Ping` uses broadcast-only fan-out on control to
// stay O(1) per fleet size (`fleet_cmd::broadcast_cmd`).

/// Max milliseconds a sniffer may stay off the control channel before Phase A returns (dwell
/// off-channel cap; command latency bound only — not GPS history retention).
pub const SNIFFER_MAX_OFF_CONTROL_MS: u64 = 3_000;

/// Passive BLE GAP scan duration per wardrive sweep (BLE role only).
pub const BLE_SCAN_WINDOW_MS: u64 = 3_000;

/// Extended `join_before_dump` deadline under bulk or age join pressure (wolfPACK fail-safe).
pub const SNIFFER_JOIN_PRESSURE_DEADLINE_MS: u64 = 10_000;

/// Control-side GPS history window for matching deferred bulk scan rows (5 minutes).
pub const GPS_HIST_RETENTION_MS: u64 = 300_000;

/// True when `scan_end_unix_ms` is set and falls within [`GPS_HIST_RETENTION_MS`] of `now_unix_ms`.
#[inline]
pub fn bulk_gps_matchable(scan_end_unix_ms: u64, now_unix_ms: u64) -> bool {
    scan_end_unix_ms > 0 && now_unix_ms.saturating_sub(scan_end_unix_ms) <= GPS_HIST_RETENTION_MS
}

/// Default per-channel promisc dwell (ms) after a successful channel switch. The sniffer starts
/// this timer in [`crate::wifi_promisc::PromiscCapture::capture_channel`] only after `set_channel`
/// returns, so driver switch time is not counted toward this on-air listen budget.
pub const DEFAULT_SCAN_DWELL_MS: u16 = 200;

/// Conservative floor (ms) per WiFi scan hop for driver `set_channel` when estimating **wall-clock**
/// Phase A duration. The sniffer maintains an internal hop aggregate (`SnifferHopAgg`); dwell
/// scheduling uses `max(this, measured_avg_per_hop)` so cold-start estimates stay safe.
pub const SNIFFER_SWEEP_SET_CHANNEL_BUDGET_MS_PER_HOP: u64 = 10;

/// Fixed marker to avoid accidentally parsing random ESP-NOW traffic as wardriver frames.
pub const MAGIC: [u8; 4] = *b"WDRV";

/// Shared fleet key. Must match between control and sniffers.
///
/// NOTE: This is a lightweight guard, not cryptographic security.
pub const FLEET_KEY: u32 = 0x3257_4452; // "RDW2" (arbitrary)

/// ESP-NOW v2 application payload limit (bytes). Matches IDF `ESP_NOW_MAX_DATA_LEN_V2`.
pub const ESP_NOW_MAX_FRAME_LEN: usize = 1470;

/// Conservative per-frame pack budget (10 B headroom under [`ESP_NOW_MAX_FRAME_LEN`]).
pub const ESP_NOW_FRAME_PAYLOAD_BUDGET: usize = 1460;

const COMMON_HDR_SIZE: usize = 1 + 1 + 4 + 4; // ver + type + magic + fleet_key

fn write_common_header(out: &mut Vec<u8>, frame_type: FrameType) {
    out.push(PROTO_VERSION);
    out.push(frame_type as u8);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FLEET_KEY.to_le_bytes());
}

/// True when local [`PROTO_VERSION`] lies in the peer's inclusive `[proto_min, proto_max]`.
#[inline]
pub fn peer_proto_accepts_us(peer_min: u8, peer_max: u8) -> bool {
    PROTO_VERSION >= peer_min && PROTO_VERSION <= peer_max
}

fn verify_common_header(b: &[u8], frame_type: FrameType) -> Result<()> {
    if b.len() < COMMON_HDR_SIZE {
        return Err(anyhow!("short frame"));
    }
    if b[0] != PROTO_VERSION || b[1] != frame_type as u8 {
        return Err(anyhow!("wrong ver/type"));
    }
    if &b[2..6] != MAGIC.as_slice() {
        return Err(anyhow!("bad magic"));
    }
    let k_slice: [u8; 4] = b
        .get(6..10)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| anyhow!("short fleet_key"))?;
    let k = u32::from_le_bytes(k_slice);
    if k != FLEET_KEY {
        return Err(anyhow!("bad key"));
    }
    Ok(())
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameType {
    ControlCmd = 1,
    SnifferAck = 2,
    ScanFrag = 3,
    /// Merged BSSID denylist for WiGLE privacy (≤32 MACs, one ESP-NOW frame).
    BssidDenylist = 4,
    /// Control→sniffer time sync ping (control monotonic ms + sequence).
    TimeSyncPing = 5,
    /// Sniffer→control time sync pong (sniffer monotonic ms + sniffer_id + sequence).
    TimeSyncPong = 6,
    /// Sniffer→control BLE observation batch (fragmented).
    BleScanFrag = 7,
    /// Sniffer→control probe-request burst summaries (fragmented).
    ProbeReqFrag = 8,
    /// Reserved: legacy `Ieee154ScanFrag` (removed in `PROTO_VERSION` 16; do not emit).
    ReservedLegacy9 = 9,
    /// Control→sniffer: set sniffer UTC wall clock (Unix ms) derived from control GPS time.
    TimeSet = 10,
    /// Control→sniffer: start/stop recon capture job (see [`ReconCmdV1`]).
    ReconCmd = 11,
    /// Sniffer→control: fragmented raw WiFi MPDU (recon).
    ReconWifiFrag = 12,
    /// Sniffer→control: broadcast join request (fast pairing / discovery).
    SnifferHello = 14,
    /// Control→sniffer: unicast welcome (fast start/stop + bootstrap schedule).
    ControlWelcome = 15,
    /// Sniffer→control: immediate LPR-camera WiFi detection (ESP-NOW only).
    DetectionAlert = 16,
    /// Control→sniffer: manual SSID denylist for WiGLE privacy (≤32 SSIDs, one ESP-NOW frame).
    SsidDenylist = 17,
    /// Sniffer→control: immediate LPR-camera BLE detection (ESP-NOW only).
    DetectionBleAlert = 18,
    /// Sniffer→control: broadcast join-before-dump (PROTO v6 wolfPACK).
    PeerJoin = 19,
    /// Control→sniffer: unicast peer session ack after roster gate + `ensure_peer`.
    PeerJoinAck = 20,
    /// Control→sniffer: unicast post-dump completion (PROTO v6 wolfPACK).
    PostDumpDone = 21,
    /// Control→broadcast: peer pool admission denied (PROTO v6 wolfPACK).
    PeerJoinNack = 22,
    /// Control→sniffer: SSID watchlist for beacon alerts (same payload layout as [`SsidDenylist`]).
    SsidWatchlist = 23,
    /// Sniffer→control: immediate SSID watch beacon hit (ESP-NOW only).
    SsidWatchAlert = 24,
    /// Sniffer→control: immediate Remote ID UAS detection (ESP-NOW only).
    RemoteIdAlert = 25,
    /// Sniffer→control: non-wildcard probe observations carrying SSIDs for hidden network discovery.
    ProbeObservationFrag = 26,
}

/// Why a roster-adopted sniffer is opening an encrypted peer session (dump visit).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerJoinReason {
    DumpWifi = 0,
    DumpBle = 1,
    DumpProbe = 2,
    DetectionAlert = 3,
    PeerRefresh = 4,
    RemoteIdAlert = 5,
}

impl PeerJoinReason {
    pub fn from_wire(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::DumpWifi),
            1 => Ok(Self::DumpBle),
            2 => Ok(Self::DumpProbe),
            3 => Ok(Self::DetectionAlert),
            4 => Ok(Self::PeerRefresh),
            5 => Ok(Self::RemoteIdAlert),
            _ => Err(anyhow!("unknown peer join reason")),
        }
    }
}

/// Sniffer → broadcast: request encrypted ESP-NOW peer before a dump transport burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerJoinV1 {
    pub sniffer_id: u32,
    pub sniffer_boot_id: u32,
    pub join_reason: PeerJoinReason,
}

/// Control → unicast: peer slot assignment after roster gate and `ensure_peer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerJoinAckV1 {
    pub sniffer_id: u32,
    /// Roster slot (1-based); `0` = unknown.
    pub fleet_slot: u8,
    /// Control peer-pool epoch; sniffers ignore stale acks after local peer reset.
    pub peer_epoch: u32,
}

/// Why control rejected a roster-adopted `PeerJoin` (broadcast nack).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerJoinNackReason {
    CapFull = 0,
    NotYourTurn = 1,
}

impl PeerJoinNackReason {
    pub fn from_wire(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::CapFull),
            1 => Ok(Self::NotYourTurn),
            _ => Err(anyhow!("unknown peer join nack reason")),
        }
    }
}

/// Control → broadcast: admission denied for one sniffer's dump visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerJoinNackV1 {
    pub sniffer_id: u32,
    pub reason: PeerJoinNackReason,
    /// Suggested sniffer backoff before retrying `PeerJoin` (ms).
    pub retry_after_ms: u16,
    /// 1 = live FIFO head waiting; 2+ = depth behind head; 0 reserved.
    pub queue_position: u8,
}

/// Control → unicast: post-dump completion after flush + ack wait (PROTO v6 wolfPACK).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostDumpDoneV1 {
    pub sniffer_id: u32,
    /// Monotonic per control boot; sniffers ignore stale done frames.
    pub session_seq: u32,
    /// Low 32 bits of control TX time (`stats::now_ms`).
    pub control_tx_ms: u32,
}

/// Payload after common header for [`FrameType::PeerJoin`].
pub const PEER_JOIN_BODY_SIZE: usize = 4 + 4 + 1;
/// Payload after common header for [`FrameType::PeerJoinAck`].
pub const PEER_JOIN_ACK_BODY_SIZE: usize = 4 + 1 + 4;
/// Payload after common header for [`FrameType::PostDumpDone`].
pub const POST_DUMP_DONE_BODY_SIZE: usize = 4 + 4 + 4;
/// Payload after common header for [`FrameType::PeerJoinNack`].
pub const PEER_JOIN_NACK_BODY_SIZE: usize = 4 + 1 + 2 + 1;
pub const PEER_JOIN_FRAME_LEN: usize = COMMON_HDR_SIZE + PEER_JOIN_BODY_SIZE;
pub const PEER_JOIN_ACK_FRAME_LEN: usize = COMMON_HDR_SIZE + PEER_JOIN_ACK_BODY_SIZE;
pub const PEER_JOIN_NACK_FRAME_LEN: usize = COMMON_HDR_SIZE + PEER_JOIN_NACK_BODY_SIZE;
pub const POST_DUMP_DONE_FRAME_LEN: usize = COMMON_HDR_SIZE + POST_DUMP_DONE_BODY_SIZE;
/// Payload after common header for [`FrameType::SnifferHello`].
pub const SNIFFER_HELLO_BODY_SIZE: usize = 4 + 4 + 1 + 1 + 1 + 1 + 1 + 1;
pub const SNIFFER_HELLO_FRAME_LEN: usize = COMMON_HDR_SIZE + SNIFFER_HELLO_BODY_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnifferHelloV1 {
    pub sniffer_id: u32,
    pub sniffer_boot_id: u32,
    pub role: SnifferRole,
    pub ble_capable: u8,
    /// 1 if non-DFS 5 GHz channel tune is OK at boot.
    pub five_ghz_capable: u8,
    /// PROTO cleanup candidate: remove on next wire bump; mirrors `five_ghz_capable`.
    pub dfs_capable: u8,
    /// Inclusive minimum [`PROTO_VERSION`] this node accepts from peers.
    pub proto_min: u8,
    /// Inclusive maximum [`PROTO_VERSION`] this node accepts from peers.
    pub proto_max: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlWelcomeV1 {
    pub control_boot_id: u32,
    pub control_channel: u8,
    /// 1 when the control wants scanning now (StartScanning), else 0.
    pub desired_scanning: u8,
    /// Per-channel on-air listen time (ms); see [`SNIFFER_SWEEP_SET_CHANNEL_BUDGET_MS_PER_HOP`].
    pub dwell_ms: u16,
    pub chan_count: u8,
    pub channels: [u8; CHANNELS_MAX],
    /// Inclusive minimum [`PROTO_VERSION`] this node accepts from peers.
    pub proto_min: u8,
    /// Inclusive maximum [`PROTO_VERSION`] this node accepts from peers.
    pub proto_max: u8,
    /// Per-method WiFi detection disable mask (bit `m-1` disables method `m`, `1..=31`).
    pub detection_disable_wifi_mask: u32,
    /// 1 when wardriving capture is suppressed on sniffers; 0 = normal scanning.
    pub wardrive_suppressed: u8,
    /// Fleet slot index assigned by control roster (1-based); `0` = unknown.
    pub fleet_slot: u8,
    /// Per-method BLE detection disable mask (bit `m-32` disables method `m`, `32..=63`).
    pub detection_disable_ble_mask: u32,
    /// Per-method Remote ID disable mask (bit `m-64` Wi-Fi `64..=95`, bit `m-96` BLE `96..=127`).
    pub remote_id_disable_mask: u32,
    /// 1 = BLE-role sniffers use active LE scan (SCAN_REQ); 0 = passive (default).
    pub ble_active_scan: u8,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlCmd {
    StopScanning = 0,
    StartScanning = 1,
    SetChannelSchedule = 2,
    Ping = 3,
    /// Unicast: apply [`SnifferRole`] (`desired_role`); persists on sniffer NVS.
    SetRole = 4,
    /// Unicast: operator remove from roster; body is `release_epoch` u32 after cmd byte.
    ReleaseBinding = 6,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnifferState {
    Idle = 0,
    Scanning = 1,
    Error = 2,
    /// Passive recon capture (probe PCAP); wardriving sweep paused.
    ReconCapturing = 3,
}

/// Radio / reporting role for a sniffer node (NVS + wire).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SnifferRole {
    #[default]
    Wifi = 0,
    Ble = 1,
}

impl SnifferRole {
    pub fn from_wire(v: u8) -> Self {
        match v {
            1 => Self::Ble,
            _ => Self::Wifi,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlCmdV1 {
    pub cmd: ControlCmd,
    pub cmd_seq: u32,
    /// Control node WiFi/ESP-NOW channel. Sniffer returns to this channel to send frames.
    pub control_channel: u8,
    /// Per-channel **on-air listen** time (ms) after `set_channel` returns; driver hop latency is
    /// not included (same semantics as [`DEFAULT_SCAN_DWELL_MS`]).
    pub dwell_ms: u16,
    /// Number of scan channels in `channels`.
    pub chan_count: u8,
    /// Scan channels (only first `chan_count` used). Wire array is fixed length
    /// [`CHANNELS_MAX`]; unused trailing slots are zero-filled.
    pub channels: [u8; CHANNELS_MAX],
    /// For [`ControlCmd::SetRole`]: target role. Zero for other commands.
    pub desired_role: u8,
    /// New value on each **control node** boot so sniffers can reset session without a long timeout.
    pub control_boot_id: u32,
    /// Per-method WiFi detection disable mask (bit `m-1` disables method `m`, `1..=31`).
    pub detection_disable_wifi_mask: u32,
    /// 1 when wardriving capture is suppressed on sniffers; 0 = normal scanning.
    pub wardrive_suppressed: u8,
    /// Optional piggybacked [`TimeSetV1::seq`]; `0` = no time in this frame.
    pub time_set_seq: u32,
    /// GPS wall ms at pack time when `time_set_seq != 0` ([`TimeSetV1::ctrl_unix_ms`]).
    pub ctrl_unix_ms: u64,
    /// Per-method BLE detection disable mask (bit `m-32` disables method `m`, `32..=63`).
    pub detection_disable_ble_mask: u32,
    /// 1 when the control wants scanning now (StartScanning semantics on Ping), else 0.
    /// PROTO v6 (`wolfPACK`): trailing wire byte after `detection_disable_ble_mask`.
    pub desired_scanning: u8,
    /// PROTO v6 (`wolfPACK`): estimated admit slots on broadcast `Ping` (0..=16); else 0.
    pub peer_slots_free: u8,
    /// PROTO v6 (`wolfPACK`): live FIFO join-queue depth on broadcast `Ping`; else 0.
    pub peer_join_waiters: u8,
    /// PROTO v8: per-method Remote ID disable mask (bit `m-64` Wi-Fi `64..=95`, bit `m-96` BLE `96..=127`).
    pub remote_id_disable_mask: u32,
    /// 1 = BLE-role sniffers use active LE scan (SCAN_REQ); 0 = passive (default).
    pub ble_active_scan: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnifferAckV1 {
    pub sniffer_id: u32,
    pub ack_cmd_seq: u32,
    pub state: SnifferState,
    pub role: SnifferRole,
    /// 1 if sniffer firmware was built with `ble` (NimBLE available).
    pub ble_capable: u8,
    /// New on each **sniffer** boot so the control node can invalidate stale time-sync offsets.
    pub sniffer_boot_id: u32,
    /// Internal + SPIRAM free bytes at ACK send time (`heap_caps` sums).
    pub free_heap_bytes: u32,
    /// Internal + SPIRAM total heap bytes at ACK send time.
    pub total_heap_bytes: u32,
    /// Non-idle CPU load 0–100 from [`crate::cpu_load`] at ACK send time.
    pub cpu_load_pct: u8,
    /// 1 if this sniffer can hop to non-DFS 5 GHz (ch 36 tune OK).
    pub five_ghz_capable: u8, // NEW
    /// PROTO cleanup candidate: remove on next wire bump; mirrors `five_ghz_capable`.
    pub dfs_capable: u8, // NEW
    /// Sweep cycles since sniffer boot where wardrive bulk was captured but not delivered.
    pub bulk_lost_cycles: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMini {
    Open = 0,
    Wep = 1,
    Wpa = 2,
    Wpa2 = 3,
    WpaWpa2 = 4,
    Wpa2Ent = 5,
    Wpa3 = 6,
    Wpa2Wpa3 = 7,
    Unknown = 255,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApEntryV1 {
    pub bssid: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    pub auth: AuthMini,
    pub ssid_len: u8,
    pub ssid: [u8; 32],
    /// Sniffer monotonic time (`stats::now_ms` semantics, ms since boot) for this observation —
    /// the RX instant when this BSSID’s **winning** beacon/probe-resp (best RSSI) was captured.
    ///
    /// **Control → Unix:** when the frag header has `sniffer_scan_end_unix_ms != 0`, map with
    /// `hdr.sniffer_scan_end_unix_ms + (obs_mono_ms as i64 - hdr.sniffer_scan_end_ms as i64)`.
    /// This assumes UTC tracks monotonic over one dwell (no per-packet `gettimeofday`).
    pub obs_mono_ms: u32,
}

/// Fragment header flags on ScanFrag / BleScanFrag / ProbeReqFrag (byte after common header).
/// [`FRAG_FLAG_TRANSPORT_END`] is set on exactly one frame per control-channel transport burst
/// (one dump visit), not per reassembly group when a batch spans multiple frag groups.
pub const FRAG_FLAG_TRANSPORT_END: u8 = 0x01;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanFragV1Header {
    /// PROTO v6 (wolfPACK): transport flags after common header; [`FRAG_FLAG_TRANSPORT_END`] on dump bursts.
    pub flags: u8,
    pub sniffer_id: u32,
    pub seq: u32,
    pub scan_batch_id: u32,
    /// New on each sniffer boot; used for stale frame rejection / attribution.
    pub sniffer_boot_id: u32,
    /// Sniffer monotonic timestamp (ms since boot) after completing the channel dwell
    /// (end of the passive capture window on that channel).
    pub sniffer_scan_end_ms: u32,
    /// Sniffer UTC timestamp (Unix ms) corresponding to `sniffer_scan_end_ms` when available.
    /// Zero when the sniffer has not yet applied a trusted wall clock.
    pub sniffer_scan_end_unix_ms: u64,
    pub frag_idx: u8,
    pub frag_total: u8,
    pub ap_count: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSyncPingV1 {
    pub seq: u32,
    pub ctrl_tx_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSyncPongV1 {
    pub seq: u32,
    pub sniff_rx_ms: u32,
    pub sniffer_id: u32,
}

/// Control→sniffer: set sniffer wall clock from GPS-backed control time.
///
/// **`ctrl_unix_ms`:** Unix epoch ms at the control, adjusted **only with control-local**
/// `stats::now_ms()` between the `gettimeofday` read and `pack_time_set` so it approximates the
/// value at transmit. Sniffers must apply this with `settimeofday` and must **not** add
/// `(sniffer_mono - ctrl_tx_mono_ms32)` (those monotonic clocks are on different devices).
///
/// **`ctrl_tx_mono_ms32`:** control `stats::now_ms()` low 32 bits when the frame was packed
/// (reserved for logging / future RTT work); **ignore for clock adjustment** on v18+ sniffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSetV1 {
    pub seq: u32,
    pub ctrl_unix_ms: u64,
    pub ctrl_tx_mono_ms32: u32,
}

/// Maximum total `ScanFrag` wire length (header + AP bodies). Same as [`ESP_NOW_FRAME_PAYLOAD_BUDGET`].
pub const SCAN_FRAG_MAX_PAYLOAD: usize = ESP_NOW_FRAME_PAYLOAD_BUDGET;

/// In-memory / legacy fixed-wire AP row size (46 B on wire when SSID is always padded to 32).
pub const AP_ENTRY_SIZE: usize = 6 + 1 + 1 + 1 + 1 + 32 + 4; // + obs_mono_ms

pub const SCAN_FRAG_HDR_SIZE: usize = COMMON_HDR_SIZE + 1 + 4 + 4 + 4 + 4 + 4 + 8 + 1 + 1 + 1; // common + flags + sniffer_id+seq+batch+boot_id+scan_end_ms+scan_end_unix_ms + frag_idx+frag_total+ap_count

/// Bytes available for AP bodies inside one `ScanFrag` after the header.
pub const SCAN_FRAG_ENTRY_BUDGET: usize = SCAN_FRAG_MAX_PAYLOAD - SCAN_FRAG_HDR_SIZE;

/// Fixed fields per AP on wolfPACK variable-SSID wire (excludes `ssid[ssid_len]`).
pub const AP_ENTRY_WIRE_OVERHEAD: usize = 6 + 1 + 1 + 1 + 1 + 4; // bssid + meta + obs_mono_ms

/// Largest wolfPACK AP wire row (32-byte SSID).
pub const AP_ENTRY_WIRE_MAX: usize = AP_ENTRY_WIRE_OVERHEAD + 32;

/// Buffer / reassembly hint: max AP rows if every SSID is 32 bytes.
pub const MAX_APS_PER_FRAG_WORST_CASE: usize = SCAN_FRAG_ENTRY_BUDGET / AP_ENTRY_WIRE_MAX;

/// On-wire byte length for one AP in a variable-SSID `ScanFrag`.
pub fn ap_entry_wire_len(ap: &ApEntryV1) -> usize {
    let sl = ap.ssid_len as usize;
    AP_ENTRY_WIRE_OVERHEAD.saturating_add(sl.min(32))
}

/// On-wire byte length including optional TLV tail.
pub fn ap_entry_wire_len_with_extras(ap: &ApEntryV1, extras: &ScanEntryExtras) -> usize {
    ap_entry_wire_len(ap).saturating_add(scan_entry_tlv_wire_len(extras))
}

/// On-wire byte length for one BLE entry including optional TLV tail.
pub fn ble_entry_wire_len_with_extras(_entry: &BleEntryV1, extras: &ScanEntryExtras) -> usize {
    BLE_ENTRY_SIZE.saturating_add(scan_entry_tlv_wire_len(extras))
}

/// Greedy AP fragment index ranges, counting optional TLV tails toward the entry budget.
pub fn chunk_aps_for_scan_frag_ranges(
    aps: &[ApEntryV1],
    extras: &[ScanEntryExtras],
) -> Vec<(usize, usize)> {
    greedy_entry_chunk_ranges(aps.len(), SCAN_FRAG_ENTRY_BUDGET, |i| match extras.get(i) {
        Some(ex) => ap_entry_wire_len_with_extras(&aps[i], ex),
        None => ap_entry_wire_len(&aps[i]),
    })
}

/// Count ESP-NOW frames needed to send `aps` including optional TLV tails.
pub fn count_scan_frag_chunks_with_extras(aps: &[ApEntryV1], extras: &[ScanEntryExtras]) -> usize {
    chunk_aps_for_scan_frag_ranges(aps, extras).len()
}

fn greedy_entry_chunk_ranges(
    n: usize,
    budget: usize,
    mut wlen_at: impl FnMut(usize) -> usize,
) -> Vec<(usize, usize)> {
    let mut chunks: Vec<(usize, usize)> = Vec::new();
    if n == 0 {
        return chunks;
    }
    let mut start = 0usize;
    let mut used = 0usize;
    for i in 0..n {
        let wlen = wlen_at(i);
        // Valid rows always fit; skip corrupt rows rather than splitting them.
        if wlen > budget {
            debug_assert!(false, "entry wire_len {wlen} exceeds budget {budget}");
            continue;
        }
        if used > 0 && used.saturating_add(wlen) > budget {
            chunks.push((start, i));
            start = i;
            used = wlen;
        } else {
            used = used.saturating_add(wlen);
        }
    }
    if start < n {
        chunks.push((start, n));
    }
    chunks
}

/// BLE scan observation (compact, one per advert / device slot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BleEntryV1 {
    pub addr: [u8; 6],
    pub rssi: i8,
    /// 0 public, 1 random, etc. (Bluetooth spec–style).
    pub addr_type: u8,
    pub company_id: u16,
    /// Truncated display name (UTF-8, null-padded).
    pub name: [u8; 22],
    /// Best-effort monotonic ms when this advert was reported (GAP callback time; not PHY timestamp).
    /// Same control-side anchor mapping as [`ApEntryV1::obs_mono_ms`].
    pub obs_mono_ms: u32,
}

pub const BLE_ENTRY_SIZE: usize = 6 + 1 + 1 + 2 + 22 + 4;

/// Header for fragmented BLE batches (same layout count as WiFi scan frags).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BleScanFragV1Header {
    /// Same semantics as [`ScanFragV1Header::flags`].
    pub flags: u8,
    pub sniffer_id: u32,
    pub seq: u32,
    pub scan_batch_id: u32,
    pub sniffer_boot_id: u32,
    /// Same semantics as [`ScanFragV1Header`]: monotonic ms after the BLE sweep completes.
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
    pub frag_idx: u8,
    pub frag_total: u8,
    pub entry_count: u8,
}

pub const BLE_SCAN_FRAG_HDR_SIZE: usize = SCAN_FRAG_HDR_SIZE; // identical layout prefix
pub const BLE_SCAN_FRAG_MAX_PAYLOAD: usize = SCAN_FRAG_MAX_PAYLOAD;
pub const BLE_SCAN_FRAG_ENTRY_BUDGET: usize = BLE_SCAN_FRAG_MAX_PAYLOAD - BLE_SCAN_FRAG_HDR_SIZE;
pub const MAX_BLE_PER_FRAG: usize = BLE_SCAN_FRAG_ENTRY_BUDGET / BLE_ENTRY_SIZE;

/// Greedy BLE fragment index ranges, counting optional TLV tails toward the entry budget.
pub fn chunk_ble_scan_frag_ranges(
    entries: &[BleEntryV1],
    extras: &[ScanEntryExtras],
) -> Vec<(usize, usize)> {
    greedy_entry_chunk_ranges(
        entries.len(),
        BLE_SCAN_FRAG_ENTRY_BUDGET,
        |i| match extras.get(i) {
            Some(ex) => ble_entry_wire_len_with_extras(&entries[i], ex),
            None => BLE_ENTRY_SIZE,
        },
    )
}

/// Header for fragmented probe-request burst summaries.
///
/// Layout matches [`ScanFragV1Header`] count prefix so we can reuse sizing patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeReqFragV1Header {
    /// Same semantics as [`ScanFragV1Header::flags`].
    pub flags: u8,
    pub sniffer_id: u32,
    pub seq: u32,
    pub scan_batch_id: u32,
    pub sniffer_boot_id: u32,
    /// Sniffer monotonic timestamp (ms since boot) near the end of the capture window.
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
    pub frag_idx: u8,
    pub frag_total: u8,
    pub entry_count: u8,
}

/// One summarized burst of wildcard probe requests for a single source MAC (telemetry only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeReqBurstV1 {
    pub src: [u8; 6],
    /// Wildcard probe request count (capped at 255 on-wire).
    pub wildcard_count: u8,
    /// RF center channel for this burst sample.
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    /// Average RSSI in Q8 fixed point (signed dBm << 8).
    pub rssi_avg_q8: i16,
    /// Earliest probe in this burst (sniffer monotonic ms). Control maps to Unix with the
    /// [`ProbeReqFragV1Header`] anchor: `hdr.sniffer_scan_end_unix_ms + (first_seen_ms32 - hdr.sniffer_scan_end_ms)`.
    pub first_seen_ms32: u32,
    pub last_seen_ms32: u32,
    /// `1` if **any** wildcard probe in this burst had an IE-tag chain that matched a built-in LPR-camera
    /// primary signature (computed on the sniffer). Wire bit only — burst summary does not carry the
    /// raw signature string.
    pub primary_sig_matched: u8,
}

/// Wire size of one [`ProbeReqBurstV1`] row.
pub const PROBE_REQ_ENTRY_SIZE: usize = 6 + 1 + 1 + 1 + 1 + 2 + 4 + 4 + 1; // 21

/// One probe request observation wire entry (non-wildcard probe with SSID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeObservationWire {
    pub src_mac: [u8; 6], // Client MAC that sent the probe
    pub ssid_len: u8,     // SSID length (1-32 bytes)
    pub ssid: [u8; 32],   // SSID bytes (UTF-8; zero-padded)
    pub channel: u8,      // WiFi channel probe was received on
    pub rssi: i8,         // Received signal strength in dBm
}

pub const PROBE_OBSERVATION_WIRE_SIZE: usize = 6 + 1 + 32 + 1 + 1; // 41

/// Header for probe observation ESP-NOW frag (same layout as ScanFragV1Header for reassembly).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeObservationFragV1Header {
    pub flags: u8,
    pub frag_idx: u8,
    pub frag_total: u8,
    pub entry_count: u8,
    pub sniffer_id: u32,
    pub seq: u32,
    pub scan_batch_id: u32,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
}

pub const PROBE_OBSERVATION_FRAG_HDR_SIZE: usize = 1 + 1 + 1 + 1 + 4 + 4 + 4 + 4 + 4 + 8; // 28
pub const PROBE_OBS_FRAG_TOTAL_SIZE: usize = PROBE_OBSERVATION_FRAG_HDR_SIZE + 1; // 29 (hdr + u8 entry_count prefix)

/// Immediate LPR-camera WiFi alert (sniffer → control, ESP-NOW only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetectionAlertV1 {
    pub sniffer_id: u32,
    pub seq: u32,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
    pub src: [u8; 6],
    /// [`DetectionWifiMethod`] ID (`1..=3` today).
    pub detection_method: u8,
    pub channel: u8,
    pub rssi_min: i8,
    pub rssi_max: i8,
    pub rssi_avg_q8: i16,
    pub wildcard_count: u8,
    pub first_seen_ms32: u32,
    pub last_seen_ms32: u32,
    pub primary_sig_matched: u8,
}

/// Payload bytes after the common header for [`FrameType::DetectionAlert`].
pub const DETECTION_ALERT_BODY_SIZE: usize =
    4 + 4 + 4 + 4 + 8 + 6 + 1 + 1 + 1 + 1 + 2 + 1 + 4 + 4 + 1;

/// Immediate LPR-camera BLE alert (sniffer → control, ESP-NOW only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetectionBleAlertV1 {
    pub sniffer_id: u32,
    pub seq: u32,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
    pub src: [u8; 6],
    /// BLE LPR-camera detection method ID (`32..=63` today).
    pub detection_method: u8,
    pub company_id: u16,
    pub rssi: i8,
}

/// Payload bytes after the common header for [`FrameType::DetectionBleAlert`].
pub const BLE_DETECTION_ALERT_BODY_SIZE: usize = 4 + 4 + 4 + 4 + 8 + 6 + 1 + 2 + 1;
pub const PROBE_REQ_FRAG_HDR_SIZE: usize = SCAN_FRAG_HDR_SIZE; // identical layout prefix
pub const PROBE_REQ_FRAG_MAX_PAYLOAD: usize = SCAN_FRAG_MAX_PAYLOAD;
pub const MAX_PROBE_REQ_PER_FRAG: usize =
    (PROBE_REQ_FRAG_MAX_PAYLOAD - PROBE_REQ_FRAG_HDR_SIZE) / PROBE_REQ_ENTRY_SIZE;

/// WiGLE BSSID denylist: common header + count + 6×count (max 32 MACs → 203 bytes).
pub const BSSID_DENYLIST_HDR_SIZE: usize = COMMON_HDR_SIZE + 1;
pub const BSSID_DENYLIST_MAX_MACS: usize = 32;

pub fn pack_bssid_denylist(macs: &[[u8; 6]]) -> Result<Vec<u8>> {
    if macs.len() > BSSID_DENYLIST_MAX_MACS {
        return Err(anyhow!("denylist: too many MACs"));
    }
    let total = BSSID_DENYLIST_HDR_SIZE + macs.len() * 6;
    if total > ESP_NOW_MAX_FRAME_LEN {
        return Err(anyhow!("denylist: frame too large"));
    }
    let mut out = Vec::with_capacity(total);
    write_common_header(&mut out, FrameType::BssidDenylist);
    out.push(macs.len() as u8);
    for m in macs {
        out.extend_from_slice(m);
    }
    Ok(out)
}

pub fn unpack_bssid_denylist(b: &[u8]) -> Result<Vec<[u8; 6]>> {
    if b.len() < BSSID_DENYLIST_HDR_SIZE {
        return Err(anyhow!("short bssid denylist"));
    }
    verify_common_header(b, FrameType::BssidDenylist)?;
    let n = b[10] as usize;
    if n > BSSID_DENYLIST_MAX_MACS {
        return Err(anyhow!("denylist: invalid count"));
    }
    let need = BSSID_DENYLIST_HDR_SIZE + n * 6;
    if b.len() < need {
        return Err(anyhow!("denylist: truncated"));
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let o = BSSID_DENYLIST_HDR_SIZE + i * 6;
        let mut m = [0u8; 6];
        m.copy_from_slice(&b[o..o + 6]);
        out.push(m);
    }
    Ok(out)
}

/// WiGLE SSID denylist: common header + count + (len u8 + UTF-8 bytes)×count.
pub const SSID_DENYLIST_HDR_SIZE: usize = COMMON_HDR_SIZE + 1;
pub const SSID_DENYLIST_MAX_SSIDS: usize = 32;
pub const SSID_DENYLIST_MAX_SSID_BYTES: usize = 32;

pub fn pack_ssid_denylist(ssids: &[String]) -> Result<Vec<u8>> {
    if ssids.len() > SSID_DENYLIST_MAX_SSIDS {
        return Err(anyhow!("ssid denylist: too many SSIDs"));
    }
    let mut total = SSID_DENYLIST_HDR_SIZE;
    for s in ssids {
        let b = s.as_bytes();
        if b.is_empty() || b.len() > SSID_DENYLIST_MAX_SSID_BYTES {
            return Err(anyhow!("ssid denylist: invalid SSID len"));
        }
        total = total.saturating_add(1 + b.len());
    }
    if total > ESP_NOW_MAX_FRAME_LEN {
        return Err(anyhow!("ssid denylist: frame too large"));
    }
    let mut out = Vec::with_capacity(total);
    write_common_header(&mut out, FrameType::SsidDenylist);
    out.push(ssids.len() as u8);
    for s in ssids {
        let b = s.as_bytes();
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    Ok(out)
}

pub fn unpack_ssid_denylist(b: &[u8]) -> Result<Vec<String>> {
    if b.len() < SSID_DENYLIST_HDR_SIZE {
        return Err(anyhow!("short ssid denylist"));
    }
    verify_common_header(b, FrameType::SsidDenylist)?;
    let n = b[10] as usize;
    if n > SSID_DENYLIST_MAX_SSIDS {
        return Err(anyhow!("ssid denylist: invalid count"));
    }
    let mut out = Vec::with_capacity(n);
    let mut o = SSID_DENYLIST_HDR_SIZE;
    for _ in 0..n {
        if o >= b.len() {
            return Err(anyhow!("ssid denylist: truncated"));
        }
        let len = b[o] as usize;
        o += 1;
        if len == 0 || len > SSID_DENYLIST_MAX_SSID_BYTES {
            return Err(anyhow!("ssid denylist: invalid entry len"));
        }
        if o + len > b.len() {
            return Err(anyhow!("ssid denylist: truncated entry"));
        }
        let s = std::str::from_utf8(&b[o..o + len])
            .map_err(|_| anyhow!("ssid denylist: invalid utf8"))?
            .to_owned();
        out.push(s);
        o += len;
    }
    Ok(out)
}

/// Operator SSID watchlist: same on-wire layout as [`pack_ssid_denylist`] but distinct [`FrameType`].
pub fn pack_ssid_watchlist(ssids: &[String]) -> Result<Vec<u8>> {
    if ssids.len() > SSID_DENYLIST_MAX_SSIDS {
        return Err(anyhow!("ssid watchlist: too many SSIDs"));
    }
    let mut total = SSID_DENYLIST_HDR_SIZE;
    for s in ssids {
        let b = s.as_bytes();
        if b.is_empty() || b.len() > SSID_DENYLIST_MAX_SSID_BYTES {
            return Err(anyhow!("ssid watchlist: invalid SSID len"));
        }
        total = total.saturating_add(1 + b.len());
    }
    if total > ESP_NOW_MAX_FRAME_LEN {
        return Err(anyhow!("ssid watchlist: frame too large"));
    }
    let mut out = Vec::with_capacity(total);
    write_common_header(&mut out, FrameType::SsidWatchlist);
    out.push(ssids.len() as u8);
    for s in ssids {
        let b = s.as_bytes();
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    Ok(out)
}

pub fn unpack_ssid_watchlist(b: &[u8]) -> Result<Vec<String>> {
    if b.len() < SSID_DENYLIST_HDR_SIZE {
        return Err(anyhow!("short ssid watchlist"));
    }
    verify_common_header(b, FrameType::SsidWatchlist)?;
    let n = b[10] as usize;
    if n > SSID_DENYLIST_MAX_SSIDS {
        return Err(anyhow!("ssid watchlist: invalid count"));
    }
    let mut out = Vec::with_capacity(n);
    let mut o = SSID_DENYLIST_HDR_SIZE;
    for _ in 0..n {
        if o >= b.len() {
            return Err(anyhow!("ssid watchlist: truncated"));
        }
        let len = b[o] as usize;
        o += 1;
        if len == 0 || len > SSID_DENYLIST_MAX_SSID_BYTES {
            return Err(anyhow!("ssid watchlist: invalid entry len"));
        }
        if o + len > b.len() {
            return Err(anyhow!("ssid watchlist: truncated entry"));
        }
        let s = std::str::from_utf8(&b[o..o + len])
            .map_err(|_| anyhow!("ssid watchlist: invalid utf8"))?
            .to_owned();
        out.push(s);
        o += len;
    }
    Ok(out)
}

/// Immediate SSID watch alert (sniffer → control, ESP-NOW only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SsidWatchAlertV1 {
    pub sniffer_id: u32,
    pub sniffer_boot_id: u32,
    pub channel: u8,
    pub rssi: i8,
    pub bssid: [u8; 6],
    pub ssid: String,
}

/// Fixed prefix after common header for [`FrameType::SsidWatchAlert`] (SSID bytes follow).
pub const SSID_WATCH_ALERT_FIXED_BODY_SIZE: usize = 4 + 4 + 1 + 1 + 6 + 1;

pub fn pack_ssid_watch_alert(a: &SsidWatchAlertV1) -> Result<Vec<u8>> {
    let b = a.ssid.as_bytes();
    if b.is_empty() || b.len() > SSID_DENYLIST_MAX_SSID_BYTES {
        return Err(anyhow!("ssid watch alert: invalid SSID len"));
    }
    let total = COMMON_HDR_SIZE + SSID_WATCH_ALERT_FIXED_BODY_SIZE + b.len();
    if total > ESP_NOW_MAX_FRAME_LEN {
        return Err(anyhow!("ssid watch alert: frame too large"));
    }
    let mut out = Vec::with_capacity(total);
    write_common_header(&mut out, FrameType::SsidWatchAlert);
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.extend_from_slice(&a.sniffer_boot_id.to_le_bytes());
    out.push(a.channel);
    out.push(a.rssi as u8);
    out.extend_from_slice(&a.bssid);
    out.push(b.len() as u8);
    out.extend_from_slice(b);
    Ok(out)
}

pub fn unpack_ssid_watch_alert(b: &[u8]) -> Result<SsidWatchAlertV1> {
    let min = COMMON_HDR_SIZE + SSID_WATCH_ALERT_FIXED_BODY_SIZE;
    if b.len() < min {
        return Err(anyhow!("short ssid watch alert"));
    }
    verify_common_header(b, FrameType::SsidWatchAlert)?;
    let sniffer_id = u32::from_le_bytes(b[10..14].try_into().unwrap());
    let sniffer_boot_id = u32::from_le_bytes(b[14..18].try_into().unwrap());
    let channel = b[18];
    let rssi = b[19] as i8;
    let mut bssid = [0u8; 6];
    bssid.copy_from_slice(&b[20..26]);
    let ssid_len = b[26] as usize;
    if ssid_len == 0 || ssid_len > SSID_DENYLIST_MAX_SSID_BYTES {
        return Err(anyhow!("ssid watch alert: invalid ssid len"));
    }
    if b.len() < min + ssid_len {
        return Err(anyhow!("ssid watch alert: truncated ssid"));
    }
    let ssid = std::str::from_utf8(&b[27..27 + ssid_len])
        .map_err(|_| anyhow!("ssid watch alert: invalid utf8"))?
        .to_owned();
    Ok(SsidWatchAlertV1 {
        sniffer_id,
        sniffer_boot_id,
        channel,
        rssi,
        bssid,
        ssid,
    })
}

/// Immediate Remote ID UAS alert (sniffer → control, ESP-NOW only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteIdAlertV1 {
    pub sniffer_id: u32,
    pub seq: u32,
    pub sniffer_boot_id: u32,
    pub sniffer_scan_end_ms: u32,
    pub sniffer_scan_end_unix_ms: u64,
    pub src: [u8; 6],
    pub channel: u8,
    pub rssi: i8,
    /// Remote ID detection method ID (`64..=127`).
    pub detection_method: u8,
    /// RF domain (`0` = Wi-Fi, `1` = BLE); same encoding as detection `signal_kind`.
    pub signal_kind: u8,
    pub uas_id: [u8; 20],
    pub drone_lat_e7: i32,
    pub drone_lon_e7: i32,
    pub alt_geo_dm: u16,
    pub heading_deg: u16,
    pub speed_h_dmps: u16,
    pub operator_lat_e7: i32,
    pub operator_lon_e7: i32,
    pub flags: u8,
    // Optional tail (always emitted by current sniffers; unpack defaults when absent).
    pub id_type: u8,
    pub ua_type: u8,
    pub status: u8,
    pub height_agl_dm: u16,
    pub height_type: u8,
    pub horiz_accuracy: u8,
    pub vert_accuracy: u8,
    pub speed_accuracy: u8,
    pub baro_accuracy: u8,
    pub ts_accuracy: u8,
    pub timestamp_ds: u16,
    pub operator_location_type: u8,
    pub operator_id_type: u8,
    pub operator_id: [u8; 20],
}

/// Fixed prefix after common header (through `flags`).
pub const REMOTE_ID_ALERT_PREFIX_SIZE: usize =
    4 + 4 + 4 + 4 + 8 + 6 + 1 + 1 + 1 + 1 + 20 + 4 + 4 + 2 + 2 + 2 + 4 + 4 + 1;

/// Optional tail after `flags` (Basic ID / Location / System / Operator ID extras).
pub const REMOTE_ID_ALERT_TAIL_SIZE: usize = 2 + 1 + 2 + 1 + 3 + 2 + 1 + 1 + 20;

/// Legacy body size (prefix only); short frames unpack with tail fields zeroed.
pub const REMOTE_ID_ALERT_BODY_SIZE: usize = REMOTE_ID_ALERT_PREFIX_SIZE;

/// Full body with tail (current sniffer emit).
pub const REMOTE_ID_ALERT_BODY_WITH_TAIL_SIZE: usize =
    REMOTE_ID_ALERT_PREFIX_SIZE + REMOTE_ID_ALERT_TAIL_SIZE;

pub fn pack_remote_id_alert(a: RemoteIdAlertV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + REMOTE_ID_ALERT_BODY_WITH_TAIL_SIZE);
    write_common_header(&mut out, FrameType::RemoteIdAlert);
    pack_remote_id_alert_body(&mut out, &a);
    out
}

fn pack_remote_id_alert_body(out: &mut Vec<u8>, a: &RemoteIdAlertV1) {
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.extend_from_slice(&a.seq.to_le_bytes());
    out.extend_from_slice(&a.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_unix_ms.to_le_bytes());
    out.extend_from_slice(&a.src);
    out.push(a.channel);
    out.push(a.rssi as u8);
    out.push(a.detection_method);
    out.push(a.signal_kind);
    out.extend_from_slice(&a.uas_id);
    out.extend_from_slice(&a.drone_lat_e7.to_le_bytes());
    out.extend_from_slice(&a.drone_lon_e7.to_le_bytes());
    out.extend_from_slice(&a.alt_geo_dm.to_le_bytes());
    out.extend_from_slice(&a.heading_deg.to_le_bytes());
    out.extend_from_slice(&a.speed_h_dmps.to_le_bytes());
    out.extend_from_slice(&a.operator_lat_e7.to_le_bytes());
    out.extend_from_slice(&a.operator_lon_e7.to_le_bytes());
    out.push(a.flags);
    out.push(a.id_type);
    out.push(a.ua_type);
    out.push(a.status);
    out.extend_from_slice(&a.height_agl_dm.to_le_bytes());
    out.push(a.height_type);
    out.push(a.horiz_accuracy | (a.vert_accuracy << 4));
    out.push(a.speed_accuracy | (a.baro_accuracy << 4));
    out.push(a.ts_accuracy & 0x0F);
    out.extend_from_slice(&a.timestamp_ds.to_le_bytes());
    out.push(a.operator_location_type);
    out.push(a.operator_id_type);
    out.extend_from_slice(&a.operator_id);
}

pub fn unpack_remote_id_alert(b: &[u8]) -> Result<RemoteIdAlertV1> {
    let need = COMMON_HDR_SIZE + REMOTE_ID_ALERT_BODY_SIZE;
    if b.len() < need {
        return Err(anyhow!("short remote id alert"));
    }
    verify_common_header(b, FrameType::RemoteIdAlert)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert seq"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(18..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(22..26)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(26..34)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert scan_end_unix_ms"))?,
    );
    let mut src = [0u8; 6];
    src.copy_from_slice(
        b.get(34..40)
            .ok_or_else(|| anyhow!("short remote_id_alert src"))?,
    );
    let channel = *b
        .get(40)
        .ok_or_else(|| anyhow!("short remote_id_alert channel"))?;
    let rssi = b[41] as i8;
    let detection_method = *b
        .get(42)
        .ok_or_else(|| anyhow!("short remote_id_alert detection_method"))?;
    let signal_kind = *b
        .get(43)
        .ok_or_else(|| anyhow!("short remote_id_alert signal_kind"))?;
    let mut uas_id = [0u8; 20];
    uas_id.copy_from_slice(
        b.get(44..64)
            .ok_or_else(|| anyhow!("short remote_id_alert uas_id"))?,
    );
    let drone_lat_e7 = i32::from_le_bytes(
        b.get(64..68)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert drone_lat_e7"))?,
    );
    let drone_lon_e7 = i32::from_le_bytes(
        b.get(68..72)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert drone_lon_e7"))?,
    );
    let alt_geo_dm = u16::from_le_bytes(
        b.get(72..74)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert alt_geo_dm"))?,
    );
    let heading_deg = u16::from_le_bytes(
        b.get(74..76)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert heading_deg"))?,
    );
    let speed_h_dmps = u16::from_le_bytes(
        b.get(76..78)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert speed_h_dmps"))?,
    );
    let operator_lat_e7 = i32::from_le_bytes(
        b.get(78..82)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert operator_lat_e7"))?,
    );
    let operator_lon_e7 = i32::from_le_bytes(
        b.get(82..86)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short remote_id_alert operator_lon_e7"))?,
    );
    let flags = *b
        .get(86)
        .ok_or_else(|| anyhow!("short remote_id_alert flags"))?;
    let mut alert = RemoteIdAlertV1 {
        sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        src,
        channel,
        rssi,
        detection_method,
        signal_kind,
        uas_id,
        drone_lat_e7,
        drone_lon_e7,
        alt_geo_dm,
        heading_deg,
        speed_h_dmps,
        operator_lat_e7,
        operator_lon_e7,
        flags,
        id_type: 0,
        ua_type: 0,
        status: 0,
        height_agl_dm: 0,
        height_type: 0,
        horiz_accuracy: 0,
        vert_accuracy: 0,
        speed_accuracy: 0,
        baro_accuracy: 0,
        ts_accuracy: 0,
        timestamp_ds: 0,
        operator_location_type: 0,
        operator_id_type: 0,
        operator_id: [0; 20],
    };
    let tail_off = COMMON_HDR_SIZE + REMOTE_ID_ALERT_PREFIX_SIZE;
    if b.len() >= tail_off + REMOTE_ID_ALERT_TAIL_SIZE {
        alert.id_type = b[tail_off];
        alert.ua_type = b[tail_off + 1];
        alert.status = b[tail_off + 2];
        alert.height_agl_dm = u16::from_le_bytes([b[tail_off + 3], b[tail_off + 4]]);
        alert.height_type = b[tail_off + 5];
        let acc0 = b[tail_off + 6];
        let acc1 = b[tail_off + 7];
        alert.horiz_accuracy = acc0 & 0x0F;
        alert.vert_accuracy = acc0 >> 4;
        alert.speed_accuracy = acc1 & 0x0F;
        alert.baro_accuracy = acc1 >> 4;
        alert.ts_accuracy = b[tail_off + 8] & 0x0F;
        alert.timestamp_ds = u16::from_le_bytes([b[tail_off + 9], b[tail_off + 10]]);
        alert.operator_location_type = b[tail_off + 11];
        alert.operator_id_type = b[tail_off + 12];
        alert
            .operator_id
            .copy_from_slice(&b[tail_off + 13..tail_off + 33]);
    }
    Ok(alert)
}

// ControlCmd body after common header (v5):
//   cmd u8, cmd_seq u32, control_channel u8, dwell_ms u16, chan_count u8,
//   channels [CHANNELS_MAX], desired_role u8, control_boot_id u32,
//   detection_disable_wifi_mask u32, wardrive_suppressed u8,
//   time_set_seq u32, ctrl_unix_ms u64, detection_disable_ble_mask u32
// PROTO v6 (`wolfPACK`): + desired_scanning u8, peer_slots_free u8, peer_join_waiters u8.
// PROTO v8: + remote_id_disable_mask u32, ble_active_scan u8.
const CONTROL_CMD_BODY_V5_LEN: usize = 1 + 4 + 1 + 2 + 1 + CHANNELS_MAX + 1 + 4 + 4 + 1 + 4 + 8 + 4;

/// `ControlCmd::Ping` short body after common header (PROTO v8 + `ble_active_scan`).
const PING_CMD_BODY_LEN: usize = 1 + 4 + 1 + 4 + 4 + 1 + 4 + 8 + 4 + 1 + 1 + 1 + 4 + 1;

/// ESP-NOW `ControlCmd::Ping` short form — liveness, scan intent, join hints (no channel plan).
pub fn pack_ping_vec(cmd: ControlCmdV1) -> Result<Vec<u8>> {
    if cmd.cmd != ControlCmd::Ping {
        return Err(anyhow!("pack_ping_vec: not Ping"));
    }
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + PING_CMD_BODY_LEN);
    write_common_header(&mut out, FrameType::ControlCmd);
    out.push(ControlCmd::Ping as u8);
    out.extend_from_slice(&cmd.cmd_seq.to_le_bytes());
    out.push(cmd.control_channel);
    out.extend_from_slice(&cmd.control_boot_id.to_le_bytes());
    out.extend_from_slice(&cmd.detection_disable_wifi_mask.to_le_bytes());
    out.push(cmd.wardrive_suppressed.min(1));
    out.extend_from_slice(&cmd.time_set_seq.to_le_bytes());
    out.extend_from_slice(&cmd.ctrl_unix_ms.to_le_bytes());
    out.extend_from_slice(&cmd.detection_disable_ble_mask.to_le_bytes());
    out.push(cmd.desired_scanning.min(1));
    out.push(cmd.peer_slots_free.min(16));
    out.push(cmd.peer_join_waiters);
    out.extend_from_slice(&cmd.remote_id_disable_mask.to_le_bytes());
    out.push(cmd.ble_active_scan.min(1));
    Ok(out)
}

/// Parse ESP-NOW `ControlCmd::Ping` short form (PROTO v8 + `ble_active_scan`; fixed length).
pub fn unpack_ping(b: &[u8]) -> Result<ControlCmdV1> {
    const NEED: usize = COMMON_HDR_SIZE + PING_CMD_BODY_LEN;
    if b.len() < NEED {
        return Err(anyhow!("short ping cmd"));
    }
    verify_common_header(b, FrameType::ControlCmd)?;
    if b[10] != ControlCmd::Ping as u8 {
        return Err(anyhow!("not ping cmd"));
    }
    let cmd_seq = u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping cmd_seq"))?,
    );
    let control_channel = b[15];
    let control_boot_id = u32::from_le_bytes(
        b.get(16..20)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping control_boot_id"))?,
    );
    let detection_disable_wifi_mask = u32::from_le_bytes(
        b.get(20..24)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping detection_disable_wifi_mask"))?,
    );
    let wardrive_suppressed = *b
        .get(24)
        .ok_or_else(|| anyhow!("short ping wardrive_suppressed"))?;
    let time_set_seq = u32::from_le_bytes(
        b.get(25..29)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping time_set_seq"))?,
    );
    let ctrl_unix_ms = u64::from_le_bytes(
        b.get(29..37)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping ctrl_unix_ms"))?,
    );
    let detection_disable_ble_mask = u32::from_le_bytes(
        b.get(37..41)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping detection_disable_ble_mask"))?,
    );
    let desired_scanning = b[41].min(1);
    let peer_slots_free = b[42].min(16);
    let peer_join_waiters = *b
        .get(43)
        .ok_or_else(|| anyhow!("short ping peer_join_waiters"))?;
    let remote_id_disable_mask = u32::from_le_bytes(
        b.get(44..48)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ping remote_id_disable_mask"))?,
    );
    let ble_active_scan = *b
        .get(48)
        .ok_or_else(|| anyhow!("short ping ble_active_scan"))?;
    Ok(ControlCmdV1 {
        cmd: ControlCmd::Ping,
        cmd_seq,
        control_channel,
        dwell_ms: 0,
        chan_count: 0,
        channels: [0u8; CHANNELS_MAX],
        desired_role: 0,
        control_boot_id,
        detection_disable_wifi_mask,
        wardrive_suppressed: wardrive_suppressed.min(1),
        time_set_seq,
        ctrl_unix_ms,
        detection_disable_ble_mask,
        desired_scanning,
        peer_slots_free,
        peer_join_waiters,
        remote_id_disable_mask,
        ble_active_scan: ble_active_scan.min(1),
    })
}

pub fn pack_control_cmd_vec(cmd: ControlCmdV1) -> Result<Vec<u8>> {
    if cmd.cmd == ControlCmd::Ping {
        return Err(anyhow!("use pack_ping_vec for Ping"));
    }
    let cap = COMMON_HDR_SIZE + CONTROL_CMD_BODY_V5_LEN + 3 + 4 + 1;
    let mut out = Vec::with_capacity(cap);
    write_common_header(&mut out, FrameType::ControlCmd);
    out.push(cmd.cmd as u8);
    out.extend_from_slice(&cmd.cmd_seq.to_le_bytes());
    out.push(cmd.control_channel);
    out.extend_from_slice(&cmd.dwell_ms.to_le_bytes());
    out.push(cmd.chan_count.min(CHANNELS_MAX as u8));
    out.extend_from_slice(&cmd.channels);
    out.push(cmd.desired_role);
    out.extend_from_slice(&cmd.control_boot_id.to_le_bytes());
    out.extend_from_slice(&cmd.detection_disable_wifi_mask.to_le_bytes());
    // PROTO_VERSION 3: wardrive suppress flag follows flock mask (no prior trailing fields).
    out.push(cmd.wardrive_suppressed.min(1));
    // Trailing optional TimeSet piggyback: seq LE then ctrl_unix_ms LE (12 bytes).
    out.extend_from_slice(&cmd.time_set_seq.to_le_bytes());
    out.extend_from_slice(&cmd.ctrl_unix_ms.to_le_bytes());
    out.extend_from_slice(&cmd.detection_disable_ble_mask.to_le_bytes());
    out.push(cmd.desired_scanning.min(1));
    // Ping hints: populated by control on broadcast Ping; zero on other cmds.
    out.push(cmd.peer_slots_free.min(16));
    out.push(cmd.peer_join_waiters);
    out.extend_from_slice(&cmd.remote_id_disable_mask.to_le_bytes());
    out.push(cmd.ble_active_scan.min(1));
    Ok(out)
}

/// Piggybacked GPS time from a control command; `None` when `time_set_seq == 0`.
pub fn time_from_cmd(cmd: &ControlCmdV1) -> Option<TimeSetV1> {
    if cmd.time_set_seq == 0 {
        None
    } else {
        Some(TimeSetV1 {
            seq: cmd.time_set_seq,
            ctrl_unix_ms: cmd.ctrl_unix_ms,
            ctrl_tx_mono_ms32: 0,
        })
    }
}

/// ESP-NOW `ControlCmd::ReleaseBinding` — common header + cmd byte + `release_epoch` u32.
pub fn pack_release_binding(release_epoch: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + 5);
    write_common_header(&mut out, FrameType::ControlCmd);
    out.push(ControlCmd::ReleaseBinding as u8);
    out.extend_from_slice(&release_epoch.to_le_bytes());
    out
}

/// Parse ESP-NOW `ReleaseBinding` (`cmd` byte `6` + `release_epoch`).
pub fn unpack_release_binding(b: &[u8]) -> Result<u32> {
    const NEED: usize = COMMON_HDR_SIZE + 1 + 4;
    if b.len() < NEED {
        return Err(anyhow!("short release binding"));
    }
    verify_common_header(b, FrameType::ControlCmd)?;
    if b[10] != ControlCmd::ReleaseBinding as u8 {
        return Err(anyhow!("not release binding cmd"));
    }
    Ok(u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short release_epoch"))?,
    ))
}

pub fn unpack_control_cmd(b: &[u8]) -> Result<ControlCmdV1> {
    const NEED_V5: usize = COMMON_HDR_SIZE + CONTROL_CMD_BODY_V5_LEN;
    const NEED: usize = NEED_V5 + 3 + 4 + 1;
    if b.len() < NEED {
        return Err(anyhow!("short control cmd"));
    }
    verify_common_header(b, FrameType::ControlCmd)?;
    let cmd = match b[10] {
        0 => ControlCmd::StopScanning,
        1 => ControlCmd::StartScanning,
        2 => ControlCmd::SetChannelSchedule,
        3 => return Err(anyhow!("use unpack_ping for Ping")),
        4 => ControlCmd::SetRole,
        _ => return Err(anyhow!("unknown cmd")),
    };
    let cmd_seq = u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control cmd_seq"))?,
    );
    let control_channel = b[15];
    let dwell_ms = u16::from_le_bytes(
        b.get(16..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control dwell_ms"))?,
    );
    let chan_count = b[18].min(CHANNELS_MAX as u8);
    let mut channels = [0u8; CHANNELS_MAX];
    let ch_start = 19usize;
    let ch_end = ch_start + CHANNELS_MAX;
    channels.copy_from_slice(
        b.get(ch_start..ch_end)
            .ok_or_else(|| anyhow!("short control channels"))?,
    );
    let desired_role = b[ch_end];
    let control_boot_id = u32::from_le_bytes(
        b.get(ch_end + 1..ch_end + 5)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control boot_id"))?,
    );
    let detection_disable_wifi_mask = u32::from_le_bytes(
        b.get(ch_end + 5..ch_end + 9)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control detection_disable_wifi_mask"))?,
    );
    let wardrive_off = ch_end + 9;
    let wardrive_suppressed = *b
        .get(wardrive_off)
        .ok_or_else(|| anyhow!("short control wardrive_suppressed"))?;
    let time_set_seq = u32::from_le_bytes(
        b.get(wardrive_off + 1..wardrive_off + 5)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control time_set_seq"))?,
    );
    let ctrl_unix_ms = u64::from_le_bytes(
        b.get(wardrive_off + 5..wardrive_off + 13)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control ctrl_unix_ms"))?,
    );
    let detection_disable_ble_mask = u32::from_le_bytes(
        b.get(wardrive_off + 13..wardrive_off + 17)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control detection_disable_ble_mask"))?,
    );
    let desired_scanning = b[wardrive_off + 17].min(1);
    let peer_slots_free = b[wardrive_off + 18].min(16);
    let peer_join_waiters = *b
        .get(wardrive_off + 19)
        .ok_or_else(|| anyhow!("short control peer_join_waiters"))?;
    let remote_id_disable_mask = u32::from_le_bytes(
        b.get(wardrive_off + 20..wardrive_off + 24)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control remote_id_disable_mask"))?,
    );
    let ble_active_scan = *b
        .get(wardrive_off + 24)
        .ok_or_else(|| anyhow!("short control ble_active_scan"))?;
    Ok(ControlCmdV1 {
        cmd,
        cmd_seq,
        control_channel,
        dwell_ms,
        chan_count,
        channels,
        desired_role,
        control_boot_id,
        detection_disable_wifi_mask,
        wardrive_suppressed: wardrive_suppressed.min(1),
        time_set_seq,
        ctrl_unix_ms,
        detection_disable_ble_mask,
        desired_scanning,
        peer_slots_free,
        peer_join_waiters,
        remote_id_disable_mask,
        ble_active_scan: ble_active_scan.min(1),
    })
}

pub fn pack_sniffer_ack(a: SnifferAckV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(SNIFFER_ACK_LEN);
    write_common_header(&mut out, FrameType::SnifferAck);
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.extend_from_slice(&a.ack_cmd_seq.to_le_bytes());
    out.push(a.state as u8);
    out.push(a.role as u8);
    out.push(a.ble_capable.min(1));
    out.extend_from_slice(&a.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&a.free_heap_bytes.to_le_bytes());
    out.extend_from_slice(&a.total_heap_bytes.to_le_bytes());
    out.push(a.cpu_load_pct);
    out.push(a.five_ghz_capable.min(1)); // NEW
                                         // PROTO cleanup candidate: mirrors five_ghz_capable on the wire.
    out.push(a.dfs_capable.min(1)); // NEW
    out.extend_from_slice(&a.bulk_lost_cycles.to_le_bytes());
    out
}

/// Full SnifferAckV1 payload length.
const SNIFFER_ACK_LEN: usize = COMMON_HDR_SIZE + 4 + 4 + 1 + 1 + 1 + 4 + 4 + 4 + 1 + 1 + 1 + 4;
/// Pre-`bulk_lost_cycles` ack length; shorter frames still unpack with `bulk_lost_cycles = 0`.
const SNIFFER_ACK_LEN_PRE_BULK: usize = COMMON_HDR_SIZE + 4 + 4 + 1 + 1 + 1 + 4 + 4 + 4 + 1 + 1 + 1;

pub fn unpack_sniffer_ack(b: &[u8]) -> Result<SnifferAckV1> {
    if b.len() < SNIFFER_ACK_LEN_PRE_BULK {
        return Err(anyhow!("short sniffer ack"));
    }
    verify_common_header(b, FrameType::SnifferAck)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_ack sniffer_id"))?,
    );
    let ack_cmd_seq = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_ack ack_cmd_seq"))?,
    );
    let state = match b[18] {
        0 => SnifferState::Idle,
        1 => SnifferState::Scanning,
        2 => SnifferState::Error,
        3 => SnifferState::ReconCapturing,
        _ => SnifferState::Error,
    };
    let role = SnifferRole::from_wire(b[19]);
    let ble_capable = b[20].min(1);
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(21..25)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_ack sniffer_boot_id"))?,
    );
    let free_heap_bytes = u32::from_le_bytes(
        b.get(25..29)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_ack mem_free"))?,
    );
    let total_heap_bytes = u32::from_le_bytes(
        b.get(29..33)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_ack mem_total"))?,
    );
    let cpu_load_pct = *b
        .get(33)
        .ok_or_else(|| anyhow!("short sniffer_ack cpu_load_pct"))?;
    let five_ghz_capable = *b
        .get(34)
        .ok_or_else(|| anyhow!("short sniffer_ack five_ghz_capable"))?;
    // PROTO cleanup candidate: mirrors five_ghz_capable on the wire.
    let dfs_capable = *b
        .get(35)
        .ok_or_else(|| anyhow!("short sniffer_ack dfs_capable"))?;
    // Trailing field: absent on shorter ack frames (older peers).
    let bulk_lost_cycles = if b.len() >= SNIFFER_ACK_LEN {
        u32::from_le_bytes(
            b.get(36..40)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| anyhow!("short sniffer_ack bulk_lost_cycles"))?,
        )
    } else {
        0
    };
    Ok(SnifferAckV1 {
        sniffer_id,
        ack_cmd_seq,
        state,
        role,
        ble_capable,
        sniffer_boot_id,
        free_heap_bytes,
        total_heap_bytes,
        cpu_load_pct: cpu_load_pct.min(100),
        five_ghz_capable: five_ghz_capable.min(1), // NEW
        dfs_capable: dfs_capable.min(1),           // NEW
        bulk_lost_cycles,
    })
}

pub fn pack_sniffer_hello(h: SnifferHelloV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(SNIFFER_HELLO_FRAME_LEN);
    write_common_header(&mut out, FrameType::SnifferHello);
    out.extend_from_slice(&h.sniffer_id.to_le_bytes());
    out.extend_from_slice(&h.sniffer_boot_id.to_le_bytes());
    out.push(h.role as u8);
    out.push(h.ble_capable.min(1));
    out.push(h.five_ghz_capable.min(1));
    // PROTO cleanup candidate: mirrors five_ghz_capable on the wire.
    out.push(h.dfs_capable.min(1));
    let proto_min = if h.proto_min == 0 && h.proto_max == 0 {
        PROTO_VERSION
    } else {
        h.proto_min
    };
    let proto_max = if h.proto_min == 0 && h.proto_max == 0 {
        PROTO_VERSION
    } else {
        h.proto_max
    };
    out.push(proto_min);
    out.push(proto_max);
    out
}

pub fn unpack_sniffer_hello(b: &[u8]) -> Result<SnifferHelloV1> {
    if b.len() < SNIFFER_HELLO_FRAME_LEN {
        return Err(anyhow!("short sniffer hello"));
    }
    verify_common_header(b, FrameType::SnifferHello)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_hello sniffer_id"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short sniffer_hello sniffer_boot_id"))?,
    );
    let role = SnifferRole::from_wire(
        *b.get(18)
            .ok_or_else(|| anyhow!("short sniffer_hello role"))?,
    );
    let ble_capable = *b
        .get(19)
        .ok_or_else(|| anyhow!("short sniffer_hello ble_capable"))?;
    let five_ghz_capable = *b
        .get(20)
        .ok_or_else(|| anyhow!("short sniffer_hello five_ghz_capable"))?;
    // PROTO cleanup candidate: mirrors five_ghz_capable on the wire.
    let dfs_capable = *b
        .get(21)
        .ok_or_else(|| anyhow!("short sniffer_hello dfs_capable"))?;
    let proto_min = *b
        .get(22)
        .ok_or_else(|| anyhow!("short sniffer_hello proto_min"))?;
    let proto_max = *b
        .get(23)
        .ok_or_else(|| anyhow!("short sniffer_hello proto_max"))?;
    if !peer_proto_accepts_us(proto_min, proto_max) {
        log::warn!(
            "sniffer hello proto range [{proto_min}, {proto_max}] excludes local {PROTO_VERSION}"
        );
        return Err(anyhow!("proto version mismatch"));
    }
    Ok(SnifferHelloV1 {
        sniffer_id,
        sniffer_boot_id,
        role,
        ble_capable: ble_capable.min(1),
        five_ghz_capable: five_ghz_capable.min(1),
        dfs_capable: dfs_capable.min(1),
        proto_min,
        proto_max,
    })
}

/// Pack [`FrameType::PeerJoin`]. Wire: common(10) + sniffer_id(4) + sniffer_boot_id(4) + join_reason(1).
pub fn pack_peer_join(j: PeerJoinV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(PEER_JOIN_FRAME_LEN);
    write_common_header(&mut out, FrameType::PeerJoin);
    out.extend_from_slice(&j.sniffer_id.to_le_bytes());
    out.extend_from_slice(&j.sniffer_boot_id.to_le_bytes());
    out.push(j.join_reason as u8);
    out
}

pub fn unpack_peer_join(b: &[u8]) -> Result<PeerJoinV1> {
    if b.len() < PEER_JOIN_FRAME_LEN {
        return Err(anyhow!("short peer join"));
    }
    verify_common_header(b, FrameType::PeerJoin)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join sniffer_id"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join sniffer_boot_id"))?,
    );
    let join_reason = PeerJoinReason::from_wire(
        *b.get(18)
            .ok_or_else(|| anyhow!("short peer_join join_reason"))?,
    )?;
    Ok(PeerJoinV1 {
        sniffer_id,
        sniffer_boot_id,
        join_reason,
    })
}

/// Pack [`FrameType::PeerJoinAck`]. Wire: common(10) + sniffer_id(4) + fleet_slot(1) + peer_epoch(4).
pub fn pack_peer_join_ack(a: PeerJoinAckV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(PEER_JOIN_ACK_FRAME_LEN);
    write_common_header(&mut out, FrameType::PeerJoinAck);
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.push(a.fleet_slot);
    out.extend_from_slice(&a.peer_epoch.to_le_bytes());
    out
}

pub fn unpack_peer_join_ack(b: &[u8]) -> Result<PeerJoinAckV1> {
    if b.len() < PEER_JOIN_ACK_FRAME_LEN {
        return Err(anyhow!("short peer join ack"));
    }
    verify_common_header(b, FrameType::PeerJoinAck)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join_ack sniffer_id"))?,
    );
    let fleet_slot = *b
        .get(14)
        .ok_or_else(|| anyhow!("short peer_join_ack fleet_slot"))?;
    let peer_epoch = u32::from_le_bytes(
        b.get(15..19)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join_ack peer_epoch"))?,
    );
    Ok(PeerJoinAckV1 {
        sniffer_id,
        fleet_slot,
        peer_epoch,
    })
}

/// Pack [`FrameType::PostDumpDone`]. Wire: common(10) + sniffer_id(4) + session_seq(4) + control_tx_ms(4).
pub fn pack_post_dump_done(d: PostDumpDoneV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(POST_DUMP_DONE_FRAME_LEN);
    write_common_header(&mut out, FrameType::PostDumpDone);
    out.extend_from_slice(&d.sniffer_id.to_le_bytes());
    out.extend_from_slice(&d.session_seq.to_le_bytes());
    out.extend_from_slice(&d.control_tx_ms.to_le_bytes());
    out
}

pub fn unpack_post_dump_done(b: &[u8]) -> Result<PostDumpDoneV1> {
    if b.len() < POST_DUMP_DONE_FRAME_LEN {
        return Err(anyhow!("short post dump done"));
    }
    verify_common_header(b, FrameType::PostDumpDone)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short post_dump_done sniffer_id"))?,
    );
    let session_seq = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short post_dump_done session_seq"))?,
    );
    let control_tx_ms = u32::from_le_bytes(
        b.get(18..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short post_dump_done control_tx_ms"))?,
    );
    Ok(PostDumpDoneV1 {
        sniffer_id,
        session_seq,
        control_tx_ms,
    })
}

/// Pack [`FrameType::PeerJoinNack`]. Wire: common(10) + sniffer_id(4) + reason(1) + retry_after_ms(2) + queue_position(1).
pub fn pack_peer_join_nack(n: PeerJoinNackV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(PEER_JOIN_NACK_FRAME_LEN);
    write_common_header(&mut out, FrameType::PeerJoinNack);
    out.extend_from_slice(&n.sniffer_id.to_le_bytes());
    out.push(n.reason as u8);
    out.extend_from_slice(&n.retry_after_ms.to_le_bytes());
    out.push(n.queue_position);
    out
}

pub fn unpack_peer_join_nack(b: &[u8]) -> Result<PeerJoinNackV1> {
    if b.len() < PEER_JOIN_NACK_FRAME_LEN {
        return Err(anyhow!("short peer join nack"));
    }
    verify_common_header(b, FrameType::PeerJoinNack)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join_nack sniffer_id"))?,
    );
    let reason = PeerJoinNackReason::from_wire(
        *b.get(14)
            .ok_or_else(|| anyhow!("short peer_join_nack reason"))?,
    )?;
    let retry_after_ms = u16::from_le_bytes(
        b.get(15..17)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short peer_join_nack retry_after_ms"))?,
    );
    let queue_position = *b
        .get(17)
        .ok_or_else(|| anyhow!("short peer_join_nack queue_position"))?;
    Ok(PeerJoinNackV1 {
        sniffer_id,
        reason,
        retry_after_ms,
        queue_position,
    })
}

pub fn pack_control_welcome(w: ControlWelcomeV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        COMMON_HDR_SIZE + 4 + 1 + 1 + 2 + 1 + CHANNELS_MAX + 2 + 4 + 1 + 1 + 4 + 4 + 1,
    );
    write_common_header(&mut out, FrameType::ControlWelcome);
    out.extend_from_slice(&w.control_boot_id.to_le_bytes());
    out.push(w.control_channel.max(1));
    out.push(w.desired_scanning.min(1));
    out.extend_from_slice(&w.dwell_ms.to_le_bytes());
    out.push(w.chan_count.min(CHANNELS_MAX as u8));
    out.extend_from_slice(&w.channels);
    let proto_min = if w.proto_min == 0 && w.proto_max == 0 {
        PROTO_VERSION
    } else {
        w.proto_min
    };
    let proto_max = if w.proto_min == 0 && w.proto_max == 0 {
        PROTO_VERSION
    } else {
        w.proto_max
    };
    out.push(proto_min);
    out.push(proto_max);
    out.extend_from_slice(&w.detection_disable_wifi_mask.to_le_bytes());
    // PROTO_VERSION 3: wardrive suppress flag follows flock mask (no prior trailing fields).
    out.push(w.wardrive_suppressed.min(1));
    out.push(w.fleet_slot);
    out.extend_from_slice(&w.detection_disable_ble_mask.to_le_bytes());
    out.extend_from_slice(&w.remote_id_disable_mask.to_le_bytes());
    out.push(w.ble_active_scan.min(1));
    out
}

pub fn unpack_control_welcome(b: &[u8]) -> Result<ControlWelcomeV1> {
    // Fixed PROTO v8 body through flock WiFi mask + wardrive + fleet_slot + ble mask + remote_id + ble_active_scan.
    let need = COMMON_HDR_SIZE + 4 + 1 + 1 + 2 + 1 + CHANNELS_MAX + 2 + 4 + 1 + 1 + 4 + 4 + 1;
    if b.len() < need {
        return Err(anyhow!("short control welcome"));
    }
    verify_common_header(b, FrameType::ControlWelcome)?;
    let control_boot_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control_welcome boot_id"))?,
    );
    let control_channel = *b
        .get(14)
        .ok_or_else(|| anyhow!("short control_welcome control_channel"))?;
    let desired_scanning = *b
        .get(15)
        .ok_or_else(|| anyhow!("short control_welcome desired_scanning"))?;
    let dwell_ms = u16::from_le_bytes(
        b.get(16..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control_welcome dwell_ms"))?,
    );
    let chan_count = *b
        .get(18)
        .ok_or_else(|| anyhow!("short control_welcome chan_count"))?;
    let mut channels = [0u8; CHANNELS_MAX];
    let ch_start = 19usize;
    let ch_end = ch_start + CHANNELS_MAX;
    channels.copy_from_slice(
        b.get(ch_start..ch_end)
            .ok_or_else(|| anyhow!("short control_welcome channels"))?,
    );
    let proto_min = *b
        .get(ch_end)
        .ok_or_else(|| anyhow!("short control_welcome proto_min"))?;
    let proto_max = *b
        .get(ch_end + 1)
        .ok_or_else(|| anyhow!("short control_welcome proto_max"))?;
    if !peer_proto_accepts_us(proto_min, proto_max) {
        log::warn!(
            "control welcome proto range [{proto_min}, {proto_max}] excludes local {PROTO_VERSION}"
        );
        return Err(anyhow!("proto version mismatch"));
    }
    let detection_disable_wifi_mask = u32::from_le_bytes(
        b.get(ch_end + 2..ch_end + 6)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control_welcome detection_disable_wifi_mask"))?,
    );
    let wardrive_suppressed = *b
        .get(ch_end + 6)
        .ok_or_else(|| anyhow!("short control_welcome wardrive_suppressed"))?;
    let fleet_slot = b[ch_end + 7];
    let detection_disable_ble_mask = u32::from_le_bytes(
        b.get(ch_end + 8..ch_end + 12)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control_welcome detection_disable_ble_mask"))?,
    );
    let remote_id_disable_mask = u32::from_le_bytes(
        b.get(ch_end + 12..ch_end + 16)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short control_welcome remote_id_disable_mask"))?,
    );
    let ble_active_scan = *b
        .get(ch_end + 16)
        .ok_or_else(|| anyhow!("short control_welcome ble_active_scan"))?;
    Ok(ControlWelcomeV1 {
        control_boot_id,
        control_channel: control_channel.max(1),
        desired_scanning: desired_scanning.min(1),
        dwell_ms,
        chan_count: chan_count.min(CHANNELS_MAX as u8),
        channels,
        proto_min,
        proto_max,
        detection_disable_wifi_mask,
        wardrive_suppressed: wardrive_suppressed.min(1),
        fleet_slot,
        detection_disable_ble_mask,
        remote_id_disable_mask,
        ble_active_scan: ble_active_scan.min(1),
    })
}

/// Dump-frag `flags` wire byte (offset 10).
#[inline]
fn pack_frag_header_flags(flags: u8) -> u8 {
    flags
}

#[inline]
fn unpack_frag_header_flags(b: &[u8]) -> u8 {
    b[10]
}

pub fn pack_scan_frag(header: ScanFragV1Header, aps: &[ApEntryV1]) -> Result<Vec<u8>> {
    if aps.len() > 255 || aps.len() != header.ap_count as usize {
        return Err(anyhow!("ap_count mismatch"));
    }
    let extras = vec![ScanEntryExtras::default(); aps.len()];
    pack_scan_frag_with_extras(header, aps, &extras)
}

/// Pack WiFi scan fragment with optional per-AP TLV tails.
pub fn pack_scan_frag_with_extras(
    header: ScanFragV1Header,
    aps: &[ApEntryV1],
    extras: &[ScanEntryExtras],
) -> Result<Vec<u8>> {
    if aps.len() > 255 || aps.len() != header.ap_count as usize {
        return Err(anyhow!("ap_count mismatch"));
    }
    if aps.len() != extras.len() {
        return Err(anyhow!("extras len mismatch"));
    }
    pack_scan_frag_variable_ssid(header, aps, extras)
}

/// wolfPACK: variable-length SSID per AP (no 32-byte pad on wire).
fn pack_scan_frag_variable_ssid(
    header: ScanFragV1Header,
    aps: &[ApEntryV1],
    extras: &[ScanEntryExtras],
) -> Result<Vec<u8>> {
    let body_budget: usize = aps
        .iter()
        .zip(extras.iter())
        .map(|(ap, ex)| ap_entry_wire_len_with_extras(ap, ex))
        .sum();
    if SCAN_FRAG_HDR_SIZE.saturating_add(body_budget) > SCAN_FRAG_MAX_PAYLOAD {
        return Err(anyhow!("scan frag over budget"));
    }
    let mut out = Vec::with_capacity(SCAN_FRAG_HDR_SIZE + body_budget);
    write_scan_frag_header(&mut out, header);
    for (ap, ex) in aps.iter().zip(extras.iter()) {
        if ap.ssid_len as usize > 32 {
            return Err(anyhow!("ssid_len too large"));
        }
        out.extend_from_slice(&ap.bssid);
        out.push(ap.channel);
        out.push(ap.rssi as u8);
        out.push(ap.auth as u8);
        out.push(ap.ssid_len);
        let sl = ap.ssid_len as usize;
        out.extend_from_slice(&ap.ssid[..sl]);
        out.extend_from_slice(&ap.obs_mono_ms.to_le_bytes());
        pack_scan_entry_tlv(ex, &mut out);
    }
    debug_assert!(out.len() <= ESP_NOW_MAX_FRAME_LEN);
    Ok(out)
}

/// Shared `ScanFrag` header bytes after [`write_common_header`].
fn write_scan_frag_header(out: &mut Vec<u8>, header: ScanFragV1Header) {
    write_common_header(out, FrameType::ScanFrag);
    out.push(pack_frag_header_flags(header.flags));
    out.extend_from_slice(&header.sniffer_id.to_le_bytes());
    out.extend_from_slice(&header.seq.to_le_bytes());
    out.extend_from_slice(&header.scan_batch_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_unix_ms.to_le_bytes());
    out.push(header.frag_idx);
    out.push(header.frag_total);
    out.push(header.ap_count);
}

pub fn unpack_scan_frag(b: &[u8]) -> Result<(ScanFragV1Header, Vec<ApEntryV1>)> {
    let (header, aps, _extras) = unpack_scan_frag_with_extras(b)?;
    Ok((header, aps))
}

/// Unpack WiFi scan fragment including optional per-AP TLV tails.
pub fn unpack_scan_frag_with_extras(
    b: &[u8],
) -> Result<(ScanFragV1Header, Vec<ApEntryV1>, Vec<ScanEntryExtras>)> {
    if b.len() < SCAN_FRAG_HDR_SIZE {
        return Err(anyhow!("short scan frag"));
    }
    verify_common_header(b, FrameType::ScanFrag)?;
    let header = parse_scan_frag_header(b)?;
    let (aps, extras) = parse_scan_frag_aps_variable_with_extras(b, header.ap_count)?;
    if aps.len() != header.ap_count as usize {
        return Err(anyhow!("ap_count parse mismatch"));
    }
    Ok((header, aps, extras))
}

/// Parse `ScanFrag` header fields (common header already verified).
fn parse_scan_frag_header(b: &[u8]) -> Result<ScanFragV1Header> {
    let flags = unpack_frag_header_flags(b);
    let sniffer_id = u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(15..19)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag seq"))?,
    );
    let scan_batch_id = u32::from_le_bytes(
        b.get(19..23)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag scan_batch_id"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(23..27)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(27..31)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(31..39)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short scan_frag scan_end_unix_ms"))?,
    );
    let frag_idx = b[39];
    let frag_total = b[40];
    let ap_count = b[41];
    Ok(ScanFragV1Header {
        flags,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        frag_idx,
        frag_total,
        ap_count,
    })
}

fn parse_scan_frag_aps_variable_with_extras(
    b: &[u8],
    ap_count: u8,
) -> Result<(Vec<ApEntryV1>, Vec<ScanEntryExtras>)> {
    let mut aps = Vec::with_capacity(ap_count as usize);
    let mut extras = Vec::with_capacity(ap_count as usize);
    let mut i = SCAN_FRAG_HDR_SIZE;
    for idx in 0..ap_count {
        if i + AP_ENTRY_WIRE_OVERHEAD > b.len() {
            return Err(anyhow!("truncated ap entries"));
        }
        let mut bssid = [0u8; 6];
        bssid.copy_from_slice(&b[i..i + 6]);
        i += 6;
        let channel = b[i];
        i += 1;
        let rssi = b[i] as i8;
        i += 1;
        let auth = auth_mini_from_wire(b[i]);
        i += 1;
        let ssid_len = b[i];
        i += 1;
        if ssid_len as usize > 32 {
            return Err(anyhow!("invalid ssid_len"));
        }
        let sl = ssid_len as usize;
        if i + sl + 4 > b.len() {
            return Err(anyhow!("truncated ssid or obs_mono_ms"));
        }
        let mut ssid = [0u8; 32];
        ssid[..sl].copy_from_slice(&b[i..i + sl]);
        i += sl;
        let obs_mono_ms = u32::from_le_bytes(
            b.get(i..i + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| anyhow!("short ap obs_mono_ms"))?,
        );
        i += 4;
        aps.push(ApEntryV1 {
            bssid,
            channel,
            rssi,
            auth,
            ssid_len,
            ssid,
            obs_mono_ms,
        });
        extras.push(parse_wifi_ap_entry_tlv(b, &mut i, idx + 1 < ap_count));
    }
    if i != b.len() {
        return Err(anyhow!("trailing scan frag bytes"));
    }
    Ok((aps, extras))
}

fn auth_mini_from_wire(v: u8) -> AuthMini {
    match v {
        0 => AuthMini::Open,
        1 => AuthMini::Wep,
        2 => AuthMini::Wpa,
        3 => AuthMini::Wpa2,
        4 => AuthMini::WpaWpa2,
        5 => AuthMini::Wpa2Ent,
        6 => AuthMini::Wpa3,
        7 => AuthMini::Wpa2Wpa3,
        _ => AuthMini::Unknown,
    }
}

pub fn pack_ble_scan_frag(header: BleScanFragV1Header, entries: &[BleEntryV1]) -> Result<Vec<u8>> {
    if entries.len() > 255 || entries.len() != header.entry_count as usize {
        return Err(anyhow!("entry_count mismatch"));
    }
    let extras = vec![ScanEntryExtras::default(); entries.len()];
    pack_ble_scan_frag_with_extras(header, entries, &extras)
}

/// Pack BLE scan fragment with optional per-entry TLV tails.
pub fn pack_ble_scan_frag_with_extras(
    header: BleScanFragV1Header,
    entries: &[BleEntryV1],
    extras: &[ScanEntryExtras],
) -> Result<Vec<u8>> {
    if entries.len() > 255 || entries.len() != header.entry_count as usize {
        return Err(anyhow!("entry_count mismatch"));
    }
    if entries.len() != extras.len() {
        return Err(anyhow!("extras len mismatch"));
    }
    let body_budget: usize = entries
        .iter()
        .zip(extras.iter())
        .map(|(e, ex)| ble_entry_wire_len_with_extras(e, ex))
        .sum();
    if BLE_SCAN_FRAG_HDR_SIZE.saturating_add(body_budget) > BLE_SCAN_FRAG_MAX_PAYLOAD {
        return Err(anyhow!("ble scan frag over budget"));
    }
    let mut out = Vec::with_capacity(BLE_SCAN_FRAG_HDR_SIZE + body_budget);
    write_common_header(&mut out, FrameType::BleScanFrag);
    out.push(pack_frag_header_flags(header.flags));
    out.extend_from_slice(&header.sniffer_id.to_le_bytes());
    out.extend_from_slice(&header.seq.to_le_bytes());
    out.extend_from_slice(&header.scan_batch_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_unix_ms.to_le_bytes());
    out.push(header.frag_idx);
    out.push(header.frag_total);
    out.push(header.entry_count);
    for (e, ex) in entries.iter().zip(extras.iter()) {
        out.extend_from_slice(&e.addr);
        out.push(e.rssi as u8);
        out.push(e.addr_type);
        out.extend_from_slice(&e.company_id.to_le_bytes());
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&e.obs_mono_ms.to_le_bytes());
        pack_scan_entry_tlv(ex, &mut out);
    }
    Ok(out)
}

pub fn unpack_ble_scan_frag(b: &[u8]) -> Result<(BleScanFragV1Header, Vec<BleEntryV1>)> {
    let (header, entries, _extras) = unpack_ble_scan_frag_with_extras(b)?;
    Ok((header, entries))
}

/// Unpack BLE scan fragment including optional per-entry TLV tails.
pub fn unpack_ble_scan_frag_with_extras(
    b: &[u8],
) -> Result<(BleScanFragV1Header, Vec<BleEntryV1>, Vec<ScanEntryExtras>)> {
    if b.len() < BLE_SCAN_FRAG_HDR_SIZE {
        return Err(anyhow!("short ble scan frag"));
    }
    verify_common_header(b, FrameType::BleScanFrag)?;
    let flags = unpack_frag_header_flags(b);
    let sniffer_id = u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(15..19)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag seq"))?,
    );
    let scan_batch_id = u32::from_le_bytes(
        b.get(19..23)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag scan_batch_id"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(23..27)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(27..31)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(31..39)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_scan_frag scan_end_unix_ms"))?,
    );
    let frag_idx = b[39];
    let frag_total = b[40];
    let entry_count = b[41];
    let mut entries = Vec::with_capacity(entry_count as usize);
    let mut extras = Vec::with_capacity(entry_count as usize);
    let mut i = BLE_SCAN_FRAG_HDR_SIZE;
    for idx in 0..entry_count {
        if i + BLE_ENTRY_SIZE > b.len() {
            return Err(anyhow!("truncated ble entries"));
        }
        let mut addr = [0u8; 6];
        addr.copy_from_slice(&b[i..i + 6]);
        i += 6;
        let rssi = b[i] as i8;
        i += 1;
        let addr_type = b[i];
        i += 1;
        let company_id = u16::from_le_bytes([b[i], b[i + 1]]);
        i += 2;
        let mut name = [0u8; 22];
        name.copy_from_slice(&b[i..i + 22]);
        i += 22;
        let obs_mono_ms = u32::from_le_bytes(
            b.get(i..i + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| anyhow!("short ble obs_mono_ms"))?,
        );
        i += 4;
        entries.push(BleEntryV1 {
            addr,
            rssi,
            addr_type,
            company_id,
            name,
            obs_mono_ms,
        });
        extras.push(parse_ble_entry_tlv(b, &mut i, idx + 1 < entry_count));
    }
    if i != b.len() {
        return Err(anyhow!("trailing ble scan frag bytes"));
    }
    let header = BleScanFragV1Header {
        flags,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        frag_idx,
        frag_total,
        entry_count,
    };
    Ok((header, entries, extras))
}

pub fn pack_probe_req_frag(
    header: ProbeReqFragV1Header,
    entries: &[ProbeReqBurstV1],
) -> Result<Vec<u8>> {
    if entries.len() > 255 || entries.len() != header.entry_count as usize {
        return Err(anyhow!("entry_count mismatch"));
    }
    if entries.len() > MAX_PROBE_REQ_PER_FRAG {
        return Err(anyhow!("too many probe entries for one frag"));
    }
    let mut out =
        Vec::with_capacity(PROBE_REQ_FRAG_HDR_SIZE + entries.len() * PROBE_REQ_ENTRY_SIZE);
    write_common_header(&mut out, FrameType::ProbeReqFrag);
    out.push(pack_frag_header_flags(header.flags));
    out.extend_from_slice(&header.sniffer_id.to_le_bytes());
    out.extend_from_slice(&header.seq.to_le_bytes());
    out.extend_from_slice(&header.scan_batch_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_unix_ms.to_le_bytes());
    out.push(header.frag_idx);
    out.push(header.frag_total);
    out.push(header.entry_count);
    for e in entries {
        out.extend_from_slice(&e.src);
        out.push(e.wildcard_count);
        out.push(e.channel);
        out.push(e.rssi_min as u8);
        out.push(e.rssi_max as u8);
        out.extend_from_slice(&e.rssi_avg_q8.to_le_bytes());
        out.extend_from_slice(&e.first_seen_ms32.to_le_bytes());
        out.extend_from_slice(&e.last_seen_ms32.to_le_bytes());
        out.push(e.primary_sig_matched.min(1));
    }
    Ok(out)
}

pub fn unpack_probe_req_frag(b: &[u8]) -> Result<(ProbeReqFragV1Header, Vec<ProbeReqBurstV1>)> {
    if b.len() < PROBE_REQ_FRAG_HDR_SIZE {
        return Err(anyhow!("short probe req frag"));
    }
    verify_common_header(b, FrameType::ProbeReqFrag)?;
    let flags = unpack_frag_header_flags(b);
    let sniffer_id = u32::from_le_bytes(
        b.get(11..15)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(15..19)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag seq"))?,
    );
    let scan_batch_id = u32::from_le_bytes(
        b.get(19..23)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag scan_batch_id"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(23..27)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(27..31)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(31..39)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short probe_req_frag scan_end_unix_ms"))?,
    );
    let frag_idx = b[39];
    let frag_total = b[40];
    let entry_count = b[41];
    let mut entries = Vec::with_capacity(entry_count as usize);
    let mut i = PROBE_REQ_FRAG_HDR_SIZE;
    for _ in 0..entry_count {
        if i + PROBE_REQ_ENTRY_SIZE > b.len() {
            return Err(anyhow!("truncated probe entries"));
        }
        let mut src = [0u8; 6];
        src.copy_from_slice(&b[i..i + 6]);
        i += 6;
        let wildcard_count = b[i];
        i += 1;
        let channel = b[i];
        i += 1;
        let rssi_min = b[i] as i8;
        i += 1;
        let rssi_max = b[i] as i8;
        i += 1;
        let rssi_avg_q8 = i16::from_le_bytes([b[i], b[i + 1]]);
        i += 2;
        let first_seen_ms32 = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        i += 4;
        let last_seen_ms32 = u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        i += 4;
        let primary_sig_matched = b[i].min(1);
        i += 1;
        entries.push(ProbeReqBurstV1 {
            src,
            wildcard_count,
            channel,
            rssi_min,
            rssi_max,
            rssi_avg_q8,
            first_seen_ms32,
            last_seen_ms32,
            primary_sig_matched,
        });
    }
    let header = ProbeReqFragV1Header {
        flags,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        frag_idx,
        frag_total,
        entry_count,
    };
    Ok((header, entries))
}

pub fn pack_detection_alert(a: DetectionAlertV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + DETECTION_ALERT_BODY_SIZE);
    write_common_header(&mut out, FrameType::DetectionAlert);
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.extend_from_slice(&a.seq.to_le_bytes());
    out.extend_from_slice(&a.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_unix_ms.to_le_bytes());
    out.extend_from_slice(&a.src);
    out.push(a.detection_method);
    out.push(a.channel);
    out.push(a.rssi_min as u8);
    out.push(a.rssi_max as u8);
    out.extend_from_slice(&a.rssi_avg_q8.to_le_bytes());
    out.push(a.wildcard_count);
    out.extend_from_slice(&a.first_seen_ms32.to_le_bytes());
    out.extend_from_slice(&a.last_seen_ms32.to_le_bytes());
    out.push(a.primary_sig_matched.min(1));
    out
}

pub fn unpack_detection_alert(b: &[u8]) -> Result<DetectionAlertV1> {
    let need = COMMON_HDR_SIZE + DETECTION_ALERT_BODY_SIZE;
    if b.len() < need {
        return Err(anyhow!("short flock alert"));
    }
    verify_common_header(b, FrameType::DetectionAlert)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert seq"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(18..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(22..26)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(26..34)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert scan_end_unix_ms"))?,
    );
    let mut src = [0u8; 6];
    src.copy_from_slice(
        b.get(34..40)
            .ok_or_else(|| anyhow!("short flock_alert src"))?,
    );
    let detection_method = *b
        .get(40)
        .ok_or_else(|| anyhow!("short flock_alert detection_method"))?;
    let channel = *b
        .get(41)
        .ok_or_else(|| anyhow!("short flock_alert channel"))?;
    let rssi_min = b[42] as i8;
    let rssi_max = b[43] as i8;
    let rssi_avg_q8 = i16::from_le_bytes(
        b.get(44..46)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert rssi_avg_q8"))?,
    );
    let wildcard_count = *b
        .get(46)
        .ok_or_else(|| anyhow!("short flock_alert wildcard_count"))?;
    let first_seen_ms32 = u32::from_le_bytes(
        b.get(47..51)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert first_seen_ms32"))?,
    );
    let last_seen_ms32 = u32::from_le_bytes(
        b.get(51..55)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short flock_alert last_seen_ms32"))?,
    );
    let primary_sig_matched = b[55].min(1);
    Ok(DetectionAlertV1 {
        sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        src,
        detection_method,
        channel,
        rssi_min,
        rssi_max,
        rssi_avg_q8,
        wildcard_count,
        first_seen_ms32,
        last_seen_ms32,
        primary_sig_matched,
    })
}

pub fn pack_detection_ble_alert(a: DetectionBleAlertV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + BLE_DETECTION_ALERT_BODY_SIZE);
    write_common_header(&mut out, FrameType::DetectionBleAlert);
    out.extend_from_slice(&a.sniffer_id.to_le_bytes());
    out.extend_from_slice(&a.seq.to_le_bytes());
    out.extend_from_slice(&a.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&a.sniffer_scan_end_unix_ms.to_le_bytes());
    out.extend_from_slice(&a.src);
    out.push(a.detection_method);
    out.extend_from_slice(&a.company_id.to_le_bytes());
    out.push(a.rssi as u8);
    out
}

pub fn unpack_detection_ble_alert(b: &[u8]) -> Result<DetectionBleAlertV1> {
    let need = COMMON_HDR_SIZE + BLE_DETECTION_ALERT_BODY_SIZE;
    if b.len() < need {
        return Err(anyhow!("short ble flock alert"));
    }
    verify_common_header(b, FrameType::DetectionBleAlert)?;
    let sniffer_id = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert sniffer_id"))?,
    );
    let seq = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert seq"))?,
    );
    let sniffer_boot_id = u32::from_le_bytes(
        b.get(18..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert sniffer_boot_id"))?,
    );
    let sniffer_scan_end_ms = u32::from_le_bytes(
        b.get(22..26)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert scan_end_ms"))?,
    );
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(
        b.get(26..34)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert scan_end_unix_ms"))?,
    );
    let mut src = [0u8; 6];
    src.copy_from_slice(
        b.get(34..40)
            .ok_or_else(|| anyhow!("short ble_flock_alert src"))?,
    );
    let detection_method = *b
        .get(40)
        .ok_or_else(|| anyhow!("short ble_flock_alert detection_method"))?;
    let company_id = u16::from_le_bytes(
        b.get(41..43)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short ble_flock_alert company_id"))?,
    );
    let rssi = b[43] as i8;
    Ok(DetectionBleAlertV1 {
        sniffer_id,
        seq,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
        src,
        detection_method,
        company_id,
        rssi,
    })
}

pub fn pack_time_sync_ping(p: TimeSyncPingV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + 4 + 4);
    write_common_header(&mut out, FrameType::TimeSyncPing);
    out.extend_from_slice(&p.seq.to_le_bytes());
    out.extend_from_slice(&p.ctrl_tx_ms.to_le_bytes());
    out
}

pub fn unpack_time_sync_ping(b: &[u8]) -> Result<TimeSyncPingV1> {
    if b.len() < COMMON_HDR_SIZE + 4 + 4 {
        return Err(anyhow!("short time sync ping"));
    }
    verify_common_header(b, FrameType::TimeSyncPing)?;
    let seq = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_sync_ping seq"))?,
    );
    let ctrl_tx_ms = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_sync_ping ctrl_tx_ms"))?,
    );
    Ok(TimeSyncPingV1 { seq, ctrl_tx_ms })
}

pub fn pack_time_sync_pong(p: TimeSyncPongV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + 4 + 4 + 4);
    write_common_header(&mut out, FrameType::TimeSyncPong);
    out.extend_from_slice(&p.seq.to_le_bytes());
    out.extend_from_slice(&p.sniff_rx_ms.to_le_bytes());
    out.extend_from_slice(&p.sniffer_id.to_le_bytes());
    out
}

pub fn unpack_time_sync_pong(b: &[u8]) -> Result<TimeSyncPongV1> {
    if b.len() < COMMON_HDR_SIZE + 4 + 4 + 4 {
        return Err(anyhow!("short time sync pong"));
    }
    verify_common_header(b, FrameType::TimeSyncPong)?;
    let seq = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_sync_pong seq"))?,
    );
    let sniff_rx_ms = u32::from_le_bytes(
        b.get(14..18)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_sync_pong sniff_rx_ms"))?,
    );
    let sniffer_id = u32::from_le_bytes(
        b.get(18..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_sync_pong sniffer_id"))?,
    );
    Ok(TimeSyncPongV1 {
        seq,
        sniff_rx_ms,
        sniffer_id,
    })
}

/// Wrapping delta `to - from` in milliseconds (`stats::now_ms()` / `ctrl_tx_ms` semantics).
#[inline]
pub fn mono_delta_ms(from: u32, to: u32) -> u32 {
    to.wrapping_sub(from)
}

/// Symmetric one-way delay estimate from a control-side RTT measurement.
#[inline]
pub fn one_way_ms_from_rtt(rtt_ms: u32) -> u32 {
    rtt_ms / 2
}

/// Sniffer wall clock after one-way ESP-NOW delay from the control ping timestamp.
#[inline]
pub fn time_set_unix_ms(ctrl_unix_at_ping: u64, rtt_ms: u32) -> u64 {
    ctrl_unix_at_ping.saturating_add(u64::from(one_way_ms_from_rtt(rtt_ms)))
}

/// Reject RTT samples that are zero or implausibly large for fleet ESP-NOW.
#[inline]
pub fn rtt_sane(rtt_ms: u32) -> bool {
    rtt_ms > 0 && rtt_ms <= 500
}

/// After RTT TimeSet, ignore coarse ControlCmd piggyback for this long (11 min).
pub const TIME_SYNC_COARSE_SUPPRESS_MS: u64 = 660_000;

/// Reject offered wall times before ~2020-09 (matches sniffer `apply_time_set`).
pub const TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS: u64 = 1_600_000_000_000;

/// When local and offered wall clocks differ by more than this, bypass coarse RTT suppress.
///
/// Five seconds is above ESP-NOW one-way jitter but well below the 10/11 minute rate limits,
/// so sniffers recover quickly after control GPS/SNTP without waiting for the suppress window.
pub const TIME_SYNC_SKEW_FORCE_MS: u64 = 5_000;

/// True when `unix_ms` is high enough to be a real UTC epoch (not 1970 boot garbage).
#[inline]
pub fn wall_unix_plausible(unix_ms: u64) -> bool {
    unix_ms >= TIME_SYNC_MIN_PLAUSIBLE_UNIX_MS
}

/// Absolute difference between two Unix millisecond timestamps.
#[inline]
pub fn time_skew_ms(local_unix_ms: u64, offered_unix_ms: u64) -> u64 {
    local_unix_ms.abs_diff(offered_unix_ms)
}

/// True when a piggybacked ControlCmd TimeSet should be dropped (fresh RTT clock).
#[inline]
pub fn coarse_time_set_suppressed(last_rtt_time_set_ms: u64, now_ms: u64) -> bool {
    last_rtt_time_set_ms != 0
        && now_ms.saturating_sub(last_rtt_time_set_ms) < TIME_SYNC_COARSE_SUPPRESS_MS
}

/// Whether a coarse (ControlCmd piggyback) TimeSet should be queued or flushed.
///
/// Bypasses the post-RTT suppress window when the sniffer clock is still at boot epoch or
/// differs from the control offer by more than [`TIME_SYNC_SKEW_FORCE_MS`].
#[inline]
pub fn should_apply_coarse_time_set(
    last_rtt_time_set_ms: u64,
    now_ms: u64,
    local_unix_ms: u64,
    offered_unix_ms: u64,
) -> bool {
    // Epoch recovery: accept any plausible offer while local wall clock is still invalid.
    if !wall_unix_plausible(local_unix_ms) && wall_unix_plausible(offered_unix_ms) {
        return true;
    }
    // Large skew: control gained GPS after a no-op or stale RTT — take coarse immediately.
    if wall_unix_plausible(offered_unix_ms)
        && time_skew_ms(local_unix_ms, offered_unix_ms) > TIME_SYNC_SKEW_FORCE_MS
    {
        return true;
    }
    !coarse_time_set_suppressed(last_rtt_time_set_ms, now_ms)
}

pub fn pack_time_set(p: TimeSetV1) -> Vec<u8> {
    // common + seq + unix_ms + ctrl_tx_mono_ms32
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + 4 + 8 + 4);
    write_common_header(&mut out, FrameType::TimeSet);
    out.extend_from_slice(&p.seq.to_le_bytes());
    out.extend_from_slice(&p.ctrl_unix_ms.to_le_bytes());
    out.extend_from_slice(&p.ctrl_tx_mono_ms32.to_le_bytes());
    out
}

pub fn unpack_time_set(b: &[u8]) -> Result<TimeSetV1> {
    if b.len() < COMMON_HDR_SIZE + 4 + 8 + 4 {
        return Err(anyhow!("short time set"));
    }
    verify_common_header(b, FrameType::TimeSet)?;
    let seq = u32::from_le_bytes(
        b.get(10..14)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_set seq"))?,
    );
    let ctrl_unix_ms = u64::from_le_bytes(
        b.get(14..22)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_set ctrl_unix_ms"))?,
    );
    let ctrl_tx_mono_ms32 = u32::from_le_bytes(
        b.get(22..26)
            .and_then(|s| s.try_into().ok())
            .ok_or_else(|| anyhow!("short time_set ctrl_tx_mono_ms32"))?,
    );
    Ok(TimeSetV1 {
        seq,
        ctrl_unix_ms,
        ctrl_tx_mono_ms32,
    })
}

// --- Recon (v13): WiFi probe PCAP capture ---

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconCmdOp {
    Start = 0,
    Abort = 1,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconJobKind {
    /// Multi-sniffer probe request capture → pcapng on control SD.
    WifiProbePcap = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconCmdV1 {
    pub op: ReconCmdOp,
    pub job_kind: ReconJobKind,
    pub session_id: u32,
    pub duration_ms: u32,
    /// Control monotonic deadline (`stats::now_ms()` semantics).
    pub end_mono_ms: u64,
    pub dwell_ms: u16,
    /// Must match sniffer [`RECON_WIFI_CHANNEL_LIST_REVISION`].
    pub channel_list_revision: u16,
    /// Extra hop phase (sniffer may combine with `sniffer_id`); default 0.
    pub phase_offset: u8,
    pub control_channel: u8,
    pub control_boot_id: u32,
}

/// Non-DFS 2.4 + 5 GHz primary channel list revision (sniffer and control must match).
pub const RECON_WIFI_CHANNEL_LIST_REVISION: u16 = 2;

/// Ordered 2.4 GHz (1–11) then 5 GHz UNII-1 / UNII-3 non-DFS primaries (no DFS dwell).
/// Revision 2 adds DFS primaries for recon capture (ESP-NOW TX after return to control channel).
pub const RECON_WIFI_CHANNELS: &[u8] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 36, 40, 44, 48, 149, 153, 157, 161, 165,
    // DFS primaries (recon queues ESP-NOW frags until control channel):
    52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconWifiFragV1Header {
    pub session_id: u32,
    pub sniffer_id: u32,
    pub sniffer_boot_id: u32,
    pub capture_seq: u32,
    pub frag_idx: u8,
    pub frag_total: u8,
    pub flags: u8,
    pub mono_ts32: u32,
    pub rssi: i8,
    pub channel: u8,
    pub mpdu_total_len: u16,
}

/// Header through `mpdu_total_len` (inclusive); payload follows.
pub const RECON_WIFI_FRAG_HDR_SIZE: usize =
    COMMON_HDR_SIZE + 4 + 4 + 4 + 4 + 1 + 1 + 1 + 4 + 1 + 1 + 2;

/// ESP-NOW MTU (see `espnow::MAX_FRAME_LEN`); keep in sync.
pub const RECON_WIFI_FRAG_MAX_PAYLOAD: usize = ESP_NOW_MAX_FRAME_LEN - RECON_WIFI_FRAG_HDR_SIZE;

pub fn pack_recon_cmd(c: ReconCmdV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMMON_HDR_SIZE + 32);
    write_common_header(&mut out, FrameType::ReconCmd);
    out.push(c.op as u8);
    out.push(c.job_kind as u8);
    out.extend_from_slice(&c.session_id.to_le_bytes());
    out.extend_from_slice(&c.duration_ms.to_le_bytes());
    out.extend_from_slice(&c.end_mono_ms.to_le_bytes());
    out.extend_from_slice(&c.dwell_ms.to_le_bytes());
    out.extend_from_slice(&c.channel_list_revision.to_le_bytes());
    out.push(c.phase_offset);
    out.push(c.control_channel);
    out.extend_from_slice(&c.control_boot_id.to_le_bytes());
    out
}

pub fn unpack_recon_cmd(b: &[u8]) -> Result<ReconCmdV1> {
    const NEED: usize = COMMON_HDR_SIZE + 1 + 1 + 4 + 4 + 8 + 2 + 2 + 1 + 1 + 4;
    if b.len() < NEED {
        return Err(anyhow!("short recon cmd"));
    }
    verify_common_header(b, FrameType::ReconCmd)?;
    let op = match b[10] {
        0 => ReconCmdOp::Start,
        1 => ReconCmdOp::Abort,
        _ => return Err(anyhow!("unknown recon op")),
    };
    let job_kind = match b[11] {
        1 => ReconJobKind::WifiProbePcap,
        _ => return Err(anyhow!("unknown recon job_kind")),
    };
    let session_id = u32::from_le_bytes(b[12..16].try_into().unwrap());
    let duration_ms = u32::from_le_bytes(b[16..20].try_into().unwrap());
    let end_mono_ms = u64::from_le_bytes(b[20..28].try_into().unwrap());
    let dwell_ms = u16::from_le_bytes(b[28..30].try_into().unwrap());
    let channel_list_revision = u16::from_le_bytes(b[30..32].try_into().unwrap());
    let phase_offset = b[32];
    let control_channel = b[33];
    let control_boot_id = u32::from_le_bytes(b[34..38].try_into().unwrap());
    Ok(ReconCmdV1 {
        op,
        job_kind,
        session_id,
        duration_ms,
        end_mono_ms,
        dwell_ms,
        channel_list_revision,
        phase_offset,
        control_channel,
        control_boot_id,
    })
}

pub fn pack_recon_wifi_frag(h: ReconWifiFragV1Header, payload_chunk: &[u8]) -> Result<Vec<u8>> {
    if payload_chunk.len() > RECON_WIFI_FRAG_MAX_PAYLOAD {
        return Err(anyhow!("recon wifi frag payload too large"));
    }
    let mut out = Vec::with_capacity(RECON_WIFI_FRAG_HDR_SIZE + payload_chunk.len());
    write_common_header(&mut out, FrameType::ReconWifiFrag);
    out.extend_from_slice(&h.session_id.to_le_bytes());
    out.extend_from_slice(&h.sniffer_id.to_le_bytes());
    out.extend_from_slice(&h.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&h.capture_seq.to_le_bytes());
    out.push(h.frag_idx);
    out.push(h.frag_total);
    out.push(h.flags);
    out.extend_from_slice(&h.mono_ts32.to_le_bytes());
    out.push(h.rssi as u8);
    out.push(h.channel);
    out.extend_from_slice(&h.mpdu_total_len.to_le_bytes());
    out.extend_from_slice(payload_chunk);
    Ok(out)
}

pub fn unpack_recon_wifi_frag(b: &[u8]) -> Result<(ReconWifiFragV1Header, &[u8])> {
    if b.len() < RECON_WIFI_FRAG_HDR_SIZE {
        return Err(anyhow!("short recon wifi frag"));
    }
    verify_common_header(b, FrameType::ReconWifiFrag)?;
    let session_id = u32::from_le_bytes(b[10..14].try_into().unwrap());
    let sniffer_id = u32::from_le_bytes(b[14..18].try_into().unwrap());
    let sniffer_boot_id = u32::from_le_bytes(b[18..22].try_into().unwrap());
    let capture_seq = u32::from_le_bytes(b[22..26].try_into().unwrap());
    let frag_idx = b[26];
    let frag_total = b[27];
    let flags = b[28];
    let mono_ts32 = u32::from_le_bytes(b[29..33].try_into().unwrap());
    let rssi = b[33] as i8;
    let channel = b[34];
    let mpdu_total_len = u16::from_le_bytes(b[35..37].try_into().unwrap());
    let hdr = ReconWifiFragV1Header {
        session_id,
        sniffer_id,
        sniffer_boot_id,
        capture_seq,
        frag_idx,
        frag_total,
        flags,
        mono_ts32,
        rssi,
        channel,
        mpdu_total_len,
    };
    Ok((hdr, &b[RECON_WIFI_FRAG_HDR_SIZE..]))
}

/// Pack probe observation fragment (sniffer → control; non-wildcard probes with SSIDs).
/// Wire layout: common_header + frag_header_bytes(28) + entry_count(u8) + entries[N×41].
pub fn pack_probe_observation_frag(
    header: ProbeObservationFragV1Header,
    entries: &[ProbeObservationWire],
) -> Result<Vec<u8>> {
    let count = entries.len();
    let expected_size = PROBE_OBS_FRAG_TOTAL_SIZE + count * PROBE_OBSERVATION_WIRE_SIZE;
    if expected_size > ESP_NOW_FRAME_PAYLOAD_BUDGET {
        return Err(anyhow!(
            "probe_observation_frag too large: {} > {}",
            expected_size,
            ESP_NOW_FRAME_PAYLOAD_BUDGET
        ));
    }
    if header.entry_count as usize != count {
        return Err(anyhow!(
            "entry_count mismatch: header={} actual={}",
            header.entry_count,
            count
        ));
    }
    let mut out = Vec::with_capacity(expected_size);
    write_common_header(&mut out, FrameType::ProbeObservationFrag);
    // Frag header (28 bytes)
    out.push(header.flags);
    out.push(header.frag_idx);
    out.push(header.frag_total);
    out.push(header.entry_count);
    out.extend_from_slice(&header.sniffer_id.to_le_bytes());
    out.extend_from_slice(&header.seq.to_le_bytes());
    out.extend_from_slice(&header.scan_batch_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_boot_id.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_ms.to_le_bytes());
    out.extend_from_slice(&header.sniffer_scan_end_unix_ms.to_le_bytes());
    // Entry count prefix (u8)
    out.push(count as u8);
    // Entries
    for entry in entries {
        out.extend_from_slice(&entry.src_mac);
        out.push(entry.ssid_len);
        out.extend_from_slice(&entry.ssid);
        out.push(entry.channel);
        out.push(entry.rssi as u8);
    }
    Ok(out)
}

/// Unpack probe observation fragment (sniffer → control; non-wildcard probes with SSIDs).
pub fn unpack_probe_observation_frag(
    b: &[u8],
) -> Result<(ProbeObservationFragV1Header, Vec<ProbeObservationWire>)> {
    verify_common_header(b, FrameType::ProbeObservationFrag)?;
    let payload = &b[COMMON_HDR_SIZE..];
    if payload.len() < PROBE_OBSERVATION_FRAG_HDR_SIZE + 1 {
        return Err(anyhow!("probe_observation_frag too short"));
    }
    let mut off = 0usize;
    let flags = payload[off];
    off += 1;
    let frag_idx = payload[off];
    off += 1;
    let frag_total = payload[off];
    off += 1;
    let entry_count = payload[off];
    off += 1;
    let sniffer_id = u32::from_le_bytes(payload[off..off + 4].try_into()?);
    off += 4;
    let seq = u32::from_le_bytes(payload[off..off + 4].try_into()?);
    off += 4;
    let scan_batch_id = u32::from_le_bytes(payload[off..off + 4].try_into()?);
    off += 4;
    let sniffer_boot_id = u32::from_le_bytes(payload[off..off + 4].try_into()?);
    off += 4;
    let sniffer_scan_end_ms = u32::from_le_bytes(payload[off..off + 4].try_into()?);
    off += 4;
    let sniffer_scan_end_unix_ms = u64::from_le_bytes(payload[off..off + 8].try_into()?);
    off += 8;
    // entry_count prefix (u8)
    let count = payload[off] as usize;
    off += 1;
    let expected = off + count * PROBE_OBSERVATION_WIRE_SIZE;
    if expected != payload.len() {
        return Err(anyhow!(
            "probe_observation_frag size mismatch: expected {} got {}",
            expected,
            payload.len()
        ));
    }
    if count as u8 != entry_count {
        return Err(anyhow!(
            "probe_observation_frag count mismatch: prefix={} header={}",
            count,
            entry_count
        ));
    }
    let header = ProbeObservationFragV1Header {
        flags,
        frag_idx,
        frag_total,
        entry_count,
        sniffer_id,
        seq,
        scan_batch_id,
        sniffer_boot_id,
        sniffer_scan_end_ms,
        sniffer_scan_end_unix_ms,
    };
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let mut src_mac = [0u8; 6];
        src_mac.copy_from_slice(&payload[off..off + 6]);
        off += 6;
        let ssid_len = payload[off];
        off += 1;
        let mut ssid = [0u8; 32];
        ssid.copy_from_slice(&payload[off..off + 32]);
        off += 32;
        let channel = payload[off];
        off += 1;
        let rssi = payload[off] as i8;
        off += 1;
        entries.push(ProbeObservationWire {
            src_mac,
            ssid_len,
            ssid,
            channel,
            rssi,
        });
    }
    Ok((header, entries))
}
