# wolfPACK gateway

The gateway firmware replaces the ESP32 **control node** RF bridge with a minimal ESP32-C5 image: ESP-NOW wolfPACK v6 in, PackLink over **gateway SoftAP + plaintext TCP** out. Sniffers are unchanged.

## Role

| Plane | Location | Duties |
|-------|----------|--------|
| Realtime | [`wardriver-gateway/`](../wardriver-gateway/) + [`wardriver-fleet-realtime/`](../wardriver-fleet-realtime/) | Peer pool, join queue, decrypt, reassembly, downlink timing |
| Control/data | Android app | Policy, GPS, dedup, SQLite, uploads, geofence |

## Hardware

Seeed XIAO ESP32-C5, rod antenna, USB power. No GPS, SD, OLED, or buzzer.

## Build

```bash
cargo check --bin esp32-wardriver-gateway --features gateway -j 1
```

Alias: `cargo check-gateway`.

## Flash

Operator approval required (never flash/erase without an explicit user ask).

```bash
cargo flash-gateway
# or
./tools/wardriver flash gateway
```

Equivalent: `cargo run --release --bin esp32-wardriver-gateway --features gateway --config .cargo/gateway.toml`.

Gateway uses the shared [`partitions.csv`](../partitions.csv) partition table (same flash flow as control/sniffer). Fleet PMK is embedded at build time from `.fleet/espnow_pmk.bin`.

## SoftAP transport

PackLink v1 over **always-on management SoftAP** and TCP port **4710**:

| Item | Value |
|------|-------|
| WiFi | APSTA; SoftAP on `CONTROL_AP_CHANNEL` |
| IP | `192.168.71.1/24` |
| TCP | Port 4710; single phone client |
| Trust | WPA2 PSK + TOFU (`PhoneHello` pins one `phone_id`) |
| Frames | Plaintext `PLNK` inside WPA2 |

Constants and wire layout: [`docs/PACKLINK_SOFTAP.md`](PACKLINK_SOFTAP.md), [`wardriver-packlink/src/lib.rs`](../wardriver-packlink/src/lib.rs).

Gateway builds use the same ESP-IDF sdkconfig as control (`sdkconfig.defaults` + `sdkconfig.defaults.ble`, including NimBLE). The gateway image does not initialize the BLE stack at runtime.

`CONFIG_ESP_WIFI_PS_NONE=y` (in `sdkconfig.defaults.ble`) keeps the management SoftAP and ESP-NOW paths out of Wi-Fi power save.

## `packlink_trust` NVS

Namespace **`packlink_trust`** holds TOFU identity and optional SoftAP overrides:

| Key | Purpose |
|-----|---------|
| `phone_id` | Pinned phone UUID |
| `trusted` | Non-zero when a phone is registered |
| `ap_ssid` / `ap_pass` | Optional SoftAP credential overrides |

**One phone per gateway.** A second phone is rejected until `TrustForget` or operator trust reset.

TOFU NVS writes run on the gateway main loop (not the PackLink TCP pthread) because NVS from a PSRAM stack is fragile.

**Operator trust reset:** serial command `gateway trust reset`, or app **Forget gateway**.

## Modules

| Module | Path | Purpose |
|--------|------|---------|
| `run` | [`wardriver-gateway/src/run.rs`](../wardriver-gateway/src/run.rs) | WiFi APSTA, main loop |
| `gateway_softap` | [`gateway_softap.rs`](../wardriver-gateway/src/gateway_softap.rs) | SoftAP + DHCP on fleet channel |
| `espnow_bridge` | [`espnow_bridge.rs`](../wardriver-gateway/src/espnow_bridge.rs) | RX, reassembly, PackLink/spool export |
| `softap_transport` | [`softap_transport.rs`](../wardriver-gateway/src/softap_transport.rs) | TCP accept, TOFU, plaintext I/O |
| `packlink_trust` | [`packlink_trust.rs`](../wardriver-gateway/src/packlink_trust.rs) | NVS TOFU + SoftAP creds |
| `psram_spool` | [`psram_spool.rs`](../wardriver-gateway/src/psram_spool.rs) | PSRAM offline ring (1 MiB) |
| `gateway_ingress` | [`wardriver-gateway/src/gateway_ingress.rs`](../wardriver-gateway/src/gateway_ingress.rs) | Gateway adapter over shared `sniffer_ingress` |
| `sniffer_ingress` | [`wardriver-fleet-realtime/src/sniffer_ingress.rs`](../wardriver-fleet-realtime/src/sniffer_ingress.rs) | Shared Hello / PeerJoin / Ack lifecycle |
| `adoption_retry` | [`wardriver-gateway/src/adoption_retry.rs`](../wardriver-gateway/src/adoption_retry.rs) | FSM-aware welcome queue (Hello + deferred + timed retries) |

## Control parity

Gateway and control node share timing-critical wolfPACK paths in [`wardriver-fleet-realtime/`](../wardriver-fleet-realtime/). The gateway drops the control node's SD, NVS roster, and HTTP dashboard; ESP-NOW realtime behavior matches control where noted below.

### Shared modules (`wardriver-fleet-realtime`)

| Module | Source | Role |
|--------|--------|------|
| `fleet_ingress_gate` | [`fleet_ingress_gate.rs`](../wardriver-fleet-realtime/src/fleet_ingress_gate.rs) | Outer ingress roster gate: only `SnifferHello` and `PeerJoin` bypass; all other uplinks require an adopted `(sniffer_id, mac)` on the roster. Used by [`wardriver-control/src/control/ingress.rs`](../wardriver-control/src/control/ingress.rs) and [`wardriver-gateway/src/espnow_bridge.rs`](../wardriver-gateway/src/espnow_bridge.rs). |
| `fleet_tick` | [`fleet_tick.rs`](../wardriver-fleet-realtime/src/fleet_tick.rs) | Periodic fleet `Ping` and queue-hint ping cadence. Control passes `broadcast_ready: true`; gateway passes `policy_applied` from the RAM policy mirror (see below). Wired from [`wardriver-control/src/control/main_loop_tail.rs`](../wardriver-control/src/control/main_loop_tail.rs) and [`wardriver-gateway/src/fleet_loop.rs`](../wardriver-gateway/src/fleet_loop.rs). |
| `channel_plan_runtime` | [`channel_plan_runtime.rs`](../wardriver-fleet-realtime/src/channel_plan_runtime.rs) | `PlanRuntimeCache`, `rebalance_due`, and ESP [`SchedulePush`](../wardriver-fleet-realtime/src/channel_plan_runtime.rs) fan-out. Control: [`wardriver-control/src/control/channel_plan_cache.rs`](../wardriver-control/src/control/channel_plan_cache.rs). Gateway: [`wardriver-gateway/src/channel_plan_downlink.rs`](../wardriver-gateway/src/channel_plan_downlink.rs). |
| `welcome_bundle` / `welcome_downlink` | [`welcome_bundle.rs`](../wardriver-fleet-realtime/src/welcome_bundle.rs), [`welcome_downlink.rs`](../wardriver-fleet-realtime/src/welcome_downlink.rs) | Canonical eight-stage welcome order (`EnsurePeer` → `ControlWelcome` → `StartStop` → channel schedule → denylists → `SetRole`). Node hooks differ; stage sequence is shared. Control: [`wardriver-control/src/control/fleet_cmd.rs`](../wardriver-control/src/control/fleet_cmd.rs). Gateway: [`wardriver-gateway/src/policy_downlink.rs`](../wardriver-gateway/src/policy_downlink.rs). |
| `sniffer_ingress` | [`sniffer_ingress.rs`](../wardriver-fleet-realtime/src/sniffer_ingress.rs) | Shared Hello / PeerJoin / Ack lifecycle and deferred welcome triggers. |
| `gateway_downlink` | [`gateway_downlink.rs`](../wardriver-fleet-realtime/src/gateway_downlink.rs) | ESP-NOW broadcast and unicast command builders (`Ping` short wire, `FleetList`, `SetRole`, schedule unicast). |
| `peer_pool` | [`peer_pool.rs`](../wardriver-fleet-realtime/src/peer_pool.rs) | Encrypted ESP-NOW peer table, queue-hint ping, stale liveness eviction. |
| `fleet_commands` | [`fleet_commands.rs`](../wardriver-fleet-realtime/src/fleet_commands.rs) | Ping due/coalesce helpers and `ControlWelcome` / `StartStop` frame builders. |
| `gateway_adoption` | [`gateway_adoption.rs`](../wardriver-gateway/src/gateway_adoption.rs) | Host-testable pending-adopt FSM state and `SnifferStatus` adopt progress enums (gateway-only; control uses NVS adopt). |

Host tests for the shared layer: [`wardriver-fleet-realtime/tests/`](../wardriver-fleet-realtime/tests/) (`fleet_ingress_gate`, `fleet_tick`, `channel_plan_runtime`, `welcome_bundle`, `welcome_downlink`, `roster_gate`). Gateway adoption FSM tests: [`wardriver-gateway/tests/gateway_adoption.rs`](../wardriver-gateway/tests/gateway_adoption.rs).

### Intentional gateway-only differences

| Area | Gateway behavior | Control behavior |
|------|------------------|------------------|
| Roster / policy | Phone `PolicySyncV1` held in RAM ([`policy_downlink.rs`](../wardriver-gateway/src/policy_downlink.rs)); `policy_applied` tracks RAM policy content (roster or scan intent) and is not cleared on PackLink TCP disconnect. | Wardrive roster in NVS ([`fleet_roster.rs`](../wardriver-control/src/fleet_roster.rs), [`nvs_defer_queue.rs`](../wardriver-control/src/control/nvs_defer_queue.rs)); HTTP adopt API ([`routes_api.rs`](../wardriver-control/src/web/routes_api.rs)). |
| Phone uplink | PackLink SoftAP export of `ObservationBulk` and PSRAM spool replay ([`espnow_bridge.rs`](../wardriver-gateway/src/espnow_bridge.rs), [`softap_transport.rs`](../wardriver-gateway/src/softap_transport.rs)); `SnifferStatus` adoption telemetry on TCP. | Observations to SD / WiGLE; fleet state on `/api/stats`. |
| Adoption | Sniffer-bound confirm: welcome `StartStop` `cmd_seq` must echo in `SnifferAck` before adopt is confirmed ([`adoption_retry.rs`](../wardriver-gateway/src/adoption_retry.rs) + `gateway_adoption`). | Optimistic NVS adopt on roster write; welcome follows without PackLink ack tail. |
| Fleet ping gating | `broadcast_ready = policy_applied` (RAM policy mirror); PackLink TCP disconnect does not clear `policy_applied` ([`fleet_loop.rs`](../wardriver-gateway/src/fleet_loop.rs)). | `broadcast_ready: true` on the scanning path ([`main_loop_tail.rs`](../wardriver-control/src/control/main_loop_tail.rs)). |

PackLink wire layouts for `PolicySync`, `SnifferStatus`, and adoption fields: [`PACKLINK_PROTOCOL.md`](PACKLINK_PROTOCOL.md).

## Roster model

The fleet-roster concept (adopted sniffer → MAC → stable fleet slot) lives in five places, each with a distinct role and persistence:

| Location | Role | Persistence |
|----------|------|-------------|
| [`wardriver-core/src/fleet_roster.rs`](../wardriver-core/src/fleet_roster.rs) | Pure roster blob codec (encode/decode, slot assignment, adopt/remove) — host-testable, no NVS | None (data only) |
| [`wardriver-control/src/fleet_roster.rs`](../wardriver-control/src/fleet_roster.rs) | NVS-backed control roster (the control node's adopted-sniffer store) | NVS namespace `fleet_roster` (key `entries`, blob version 1) |
| [`wardriver-fleet-realtime/src/roster_gate.rs`](../wardriver-fleet-realtime/src/roster_gate.rs) | Phone `PolicySyncV1` RAM gate: `roster_allows` / `roster_fleet_slot` admission on the gateway ESP-NOW path | RAM only (phone pushes `PolicySync`) |
| [`wardriver-gateway/src/policy_downlink.rs`](../wardriver-gateway/src/policy_downlink.rs) | RAM policy mirror on the gateway, with downlink fan-out; survives reboot via [`policy_store.rs`](../wardriver-gateway/src/policy_store.rs) | RAM + NVS namespace `gateway_policy` (key `policy_v1`) |
| Android `FleetRosterEntity` ([`WardriverDatabase.kt`](../wardriver-android/app/src/main/java/net/wardriver/mobile/db/WardriverDatabase.kt)) | Phone-owned roster rows (adopted sniffers + live SoftAP status fields), source of `PolicySync` | Room SQLite table `fleet_roster` |

The phone's `fleet_roster` table is the operator-editable source of truth; it drives `PolicySync` downlink, the gateway RAM mirror (persisted to `gateway_policy` NVS), and the gateway `roster_gate` admission. On the control node, the equivalent store is the `fleet_roster` NVS namespace written from the HTTP adopt API.

## PackLink

Protocol: [`PACKLINK_SOFTAP.md`](PACKLINK_SOFTAP.md). Codec: [`wardriver-packlink`](../wardriver-packlink/).

## Security

Fleet PMK is provisioned at build time (same as control/sniffer). The gateway decrypts ESP-NOW; the phone receives structured PackLink batches only over the TOFU-gated TCP session inside WPA2. Fleet PMK never leaves the gateway.

## Offline behavior

When the phone disconnects, PackLink frames append to a **1 MiB PSRAM ring** (volatile across reset). On reconnect, `SpoolReplay` chunks drain over TCP before live `ObservationBulk` traffic resumes.

## Operator notes

1. Flash gateway firmware before connecting the Android app.
2. Phone joins fleet SoftAP (`wolfPACK` by default) → TOFU on first connect → verify `GatewayInfo` → push `policy_sync` with wardrive roster (up to 48 sniffers). Sniffer adopt is sniffer-bound over PackLink (`PolicySyncAck` + `SnifferStatus.adoption_state`); see [PACKLINK_PROTOCOL.md](PACKLINK_PROTOCOL.md).
3. Confirm `ObservationBulk` traffic during a short wardrive.
4. Disconnect phone; confirm gateway spool fills; reconnect and confirm `SpoolReplay` drains.
