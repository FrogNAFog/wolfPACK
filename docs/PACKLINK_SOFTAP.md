# PackLink over gateway SoftAP

PackLink v1 between the wolfPACK gateway and the Android app uses **plaintext TCP** on the gateway management SoftAP. WPA2 protects the airlink; trust-on-first-use (TOFU) pins one phone per gateway.

**Message layouts, clocks, and sniffer adoption:** [PACKLINK_PROTOCOL.md](PACKLINK_PROTOCOL.md).

## Transport

| Item | Value |
|------|-------|
| WiFi mode | Gateway **APSTA** (SoftAP + STA driver); sniffers use **ESP-NOW only** |
| SoftAP SSID/password | Fleet defaults from `.fleet/mgmt_ap.toml` (`wolfPACK` / `deflock123`); overridable in gateway NVS via app |
| SoftAP IP | `192.168.71.1/24` (same subnet as control management AP) |
| Channel | [`CONTROL_AP_CHANNEL`](../wardriver-proto/src/lib.rs) — aligned with ESP-NOW |
| TCP | `192.168.71.1:4710` ([`PACKLINK_TCP_PORT`](../wardriver-packlink/src/lib.rs)) |
| Clients | One phone (`SOFTAP_MAX_CONNECTIONS = 1`) |
| Records | Plaintext `PLNK` frames (no ChaCha record layer) |

## Phone dual-network UX

The Android app joins the gateway SoftAP with `WifiNetworkSpecifier` **without** claiming internet. Cellular remains the default route for maps and uploads. PackLink TCP uses `network.socketFactory` on the gateway network only.

## TOFU handshake (message types 11–15)

| Type | Direction | Purpose |
|------|-----------|---------|
| `PhoneHello` (11) | Phone → gateway | Stable `phone_id` (UUID string, max 36 bytes) |
| `TrustOk` (12) | Gateway → phone | TOFU accepted |
| `TrustReject` (13) | Gateway → phone | Wrong phone or policy reject |
| `SoftApConfig` (14) | Phone → gateway | New SSID + password (trusted phone only) |
| `TrustForget` (15) | Phone → gateway | Clear pinned phone |

**Rules:**

- First `PhoneHello` pins `phone_id` in gateway NVS (`packlink_trust`).
- Later connects must present the same `phone_id` or receive `TrustReject`.
- After `TrustOk`, gateway sends `GatewayInfo`, drains PSRAM spool via `SpoolReplay`, then live `ObservationBulk`.
- Operator reset: serial `gateway trust reset` or app **Forget gateway** (`TrustForget`).

## Session frames after TrustOk

See [PACKLINK_PROTOCOL.md](PACKLINK_PROTOCOL.md) for the full MsgType table (includes `PolicySync` / `PolicySyncAck`, `SnifferStatus`, `TimeSync`, capture, and events).

Codec: [`wardriver-packlink`](../wardriver-packlink/).

## Credential change flow

1. Operator edits SSID/password in app **Gateway** settings (saved locally).
2. When connected, app sends `SoftApConfig`; gateway writes NVS and restarts SoftAP.
3. App reconnects with stored credentials.

## Security notes

- Fleet ESP-NOW PMK never leaves the gateway.
- WPA2 PSK is the airlink secret; TOFU limits PackLink to one phone.
- Do not use `bindProcessToNetwork` globally on Android — it breaks cellular uploads.

## Implementation map

| Component | Path |
|-----------|------|
| Protocol reference | [`docs/PACKLINK_PROTOCOL.md`](PACKLINK_PROTOCOL.md) |
| Frame codec + TOFU types | [`wardriver-packlink/src/lib.rs`](../wardriver-packlink/src/lib.rs) |
| Gateway SoftAP boot | [`wardriver-gateway/src/gateway_softap.rs`](../wardriver-gateway/src/gateway_softap.rs) |
| TCP session | [`wardriver-gateway/src/softap_transport.rs`](../wardriver-gateway/src/softap_transport.rs) |
| NVS trust + SoftAP creds | [`wardriver-gateway/src/packlink_trust.rs`](../wardriver-gateway/src/packlink_trust.rs) |
| Android Wi-Fi join | [`wardriver-android/.../wifi/PackLinkWifiManager.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/wifi/PackLinkWifiManager.kt) |
| Android repository | [`GatewayWifiRepository.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/data/GatewayWifiRepository.kt) |
