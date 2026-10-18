//! Host-testable sniffer control-liveness math (no esp-idf).
//!
//! Pure max-of-two over the ESP-NOW RX timestamp and the HTTP 202 accept contact timestamp.
//! Extracted from `wardriver-sniffer/src/sniffer_espnow.rs` (`effective_control_liveness_ms`).

/// Combined control liveness: ESP-NOW RX (`last_control_rx_ms`) or HTTP 202 accept contact.
///
/// Returns the more recent (larger) of the two monotonic-ms timestamps, or `None` when the
/// sniffer has observed neither a control RX nor an accept contact.
pub fn effective_control_liveness_ms(rx: Option<u64>, contact: Option<u64>) -> Option<u64> {
    match (rx, contact) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}
