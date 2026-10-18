//! Append-only observation row (SQLite `observation` table semantics).

/// Phone-sourced cell tower row (WiGLE cell `Type` in `signal_kind`).
#[derive(Clone, Debug, PartialEq)]
pub struct CellObservation {
    pub id: i64,
    pub session_id: i64,
    pub observed_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub cell_type: &'static str,
    pub cell_key: String,
    pub network_name: String,
    pub capabilities: String,
    pub frequency: u32,
    pub rssi: i8,
    pub uploaded: bool,
}

/// One allowed sighting after dedup + CSV gates (many rows per MAC when revisited >50 m).
#[derive(Clone, Debug, PartialEq)]
pub struct Observation {
    pub id: i64,
    pub session_id: i64,
    pub sniffer_id: u32,
    pub observed_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub signal_kind: SignalKind,
    pub bssid: [u8; 6],
    pub ssid: String,
    pub channel: u8,
    pub frequency_mhz: u32,
    pub rssi: i8,
    pub auth_mode: String,
    pub rcois: String,
    pub ble_capabilities: String,
    pub uploaded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalKind {
    Wifi,
    Ble,
    ProbeReq,
}

impl SignalKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wifi => "WIFI",
            Self::Ble => "BLE",
            Self::ProbeReq => "PROBE_REQ",
        }
    }
}

/// Probe request observation: a client device probing for a (possibly hidden) network.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeObservation {
    pub id: i64,
    pub session_id: i64,
    pub sniffer_id: u32,
    pub observed_unix_ms: i64,
    pub lat: f64,
    pub lon: f64,
    pub altitude_m: Option<i32>,
    pub accuracy_m: Option<f32>,
    pub src_mac: [u8; 6],
    pub ssid: String,
    pub channel: u8,
    pub rssi: i8,
}
