# Sniffer health LED (XIAO ESP32-C5)

The sniffer firmware drives the yellow **user LED** on the Seeed XIAO ESP32-C5 (**GPIO27**, silkscreen **L**). Patterns report **fleet health only** — not scan role, channel, or dump progress.

Constants match [`wardriver-sniffer/src/sniffer_led.rs`](../wardriver-sniffer/src/sniffer_led.rs).

| Constant | Value |
|----------|-------|
| GPIO | 27 |
| Boot / reconnect grace | 30 s |
| Control stale threshold | 20 s |
| Healthy breath period | 4 s |

## Flash pattern chart

| Health state | LED pattern | Timing (one cycle) |
|--------------|-------------|-------------------|
| **Healthy** | Slow breathing | LEDC hardware fade: 2 s inhale + 2 s exhale (4 s period), driven by `sniffer-led` worker |
| **Not adopted** | Fast blink | 200 ms on, 200 ms off (repeat) |
| **No control link** | Double blink | 150 ms on, 150 ms off, 150 ms on, 900 ms off (repeat) |
| **Control stale** | Solid on | — |
| **Boot / reconnecting** | Long + double short | 500 ms on, 500 ms off, 150 ms on, 150 ms off, 150 ms on, 900 ms off (repeat) |

ASCII timing (one cycle, `█` = on, `·` = off):

```
Healthy:          ░░▒▒▓▓████▓▓▒▒░░····  (4 s fade in/out)

Not adopted:      ██··██··██··██··

No control link:  ██·██·██············

Control stale:    ████████████████████

Boot/reconnect:   █████·····██·██········
```

## When each state applies

Evaluation runs every main-loop tick. First matching row wins:

| Order | State | Condition |
|-------|--------|-----------|
| 1 | **Not adopted** | No NVS fleet binding (`bound_control_mac` unset; control never sent `ControlWelcome`) |
| 2 | **Boot / reconnecting** | Fleet-bound and inside the 30 s grace window after power-on or link recovery |
| 3 | **Control stale** | Fleet-bound, had control ESP-NOW contact before, and no control frame for **≥ 20 s** |
| 4 | **No control link** | Fleet-bound, past grace, session not established (`control_established` false), control not yet stale |
| 5 | **Healthy** | Fleet-bound, control session active, control heard within 20 s |

**Boot / reconnecting** grace restarts when:

- The sniffer boots (30 s from power-on for bound units).
- Control silence timeout clears the session.
- The link watchdog runs ESP-NOW recovery (~20 s without control liveness).

**Out of scope for this LED:** wardrive dump age, bulk buffer pressure, `SnifferState::Scanning` vs idle, Flock alerts, BLE vs WiFi role.

## Hardware notes

- **Charge LED (red, C)** is driven by the battery charger; the sniffer firmware does not use it.
- The yellow user LED is **active-low** (GPIO low = lit). Polarity is set via `LED_ACTIVE_HIGH` in `sniffer_led.rs`.
- **Healthy** uses ESP-IDF LEDC **hardware fade** on a dedicated `sniffer-led` pthread (~40 ms poll for fault patterns) so breathing stays smooth during long RF dwells.
