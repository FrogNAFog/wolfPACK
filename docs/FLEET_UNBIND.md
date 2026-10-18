# Fleet unbind (BOOT button)

Operators reset wolfPACK fleet binding in the field with a **5 second** hold on the XIAO **BOOT** button (module silkscreen **B**, GPIO28). Full NVS erase is not required.

## Control node

Hold BOOT for 5 seconds on the control XIAO:

1. Unicast `ReleaseBinding` to every MAC on the NVS fleet roster (with retries).
2. Clear the `fleet_roster` NVS namespace.
3. On the expansion base build (`board-xiao`): OLED shows an **UNBIND** countdown during the hold, then **UNBOUND** with a buzzer pattern when complete.

WiGLE credentials, home zone, channel plan, and other `wardriver` NVS keys are unchanged.

Re-adopt sniffers from `http://192.168.71.1/sniffers` after unbind.

## Gateway node

Hold BOOT for 5 seconds on the gateway XIAO:

1. Unicast `ReleaseBinding` to every MAC on the RAM policy roster.
2. Apply an empty `PolicySync` mirror (cancels adoption FSM entries).
3. Flash the user LED (GPIO27) three times on success.
4. When the phone is connected over PackLink, send `FleetUnbindNotify` (msg type 19) so the Android app clears its Room `fleet_roster` table.

`packlink_trust` (phone TOFU) is not cleared. Use **Forget gateway** in the app for that.

Re-adopt from the app **Sniffers** screen.

## Sniffer node

Hold BOOT for 5 seconds on a sniffer XIAO:

1. Clear NVS fleet binding (`bound_ctrl_mac`, fleet slot, control MAC).
2. Restart fast-join `SnifferHello` broadcasts.

The health LED switches to the fast-blink **not adopted** pattern (see [SNIFFER_HEALTH_LED.md](SNIFFER_HEALTH_LED.md)).

Use this when:

- A sniffer was bound to a different or dead control/gateway (`ReleaseBinding` from the new node cannot reach it).
- The sniffer was out of range when control/gateway cleared its roster.

## Typical full-fleet reset

1. Power the cluster (control or gateway plus sniffers in range).
2. Hold BOOT 5 s on **control** or **gateway**.
3. On any sniffer that still behaves as bound, hold BOOT 5 s on that unit.
4. Re-adopt all sniffers.

## PackLink wire

`FleetUnbindNotify` layout: empty payload, gateway → phone only. See [PACKLINK_PROTOCOL.md](PACKLINK_PROTOCOL.md).
