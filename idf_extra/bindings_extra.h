#pragma once
#include <stdint.h>
#include "driver/temperature_sensor.h"
#include "esp_pm.h"

/** FreeRTOS run-time stats delta (call ~1 Hz); first call(s) may return 0. */
uint8_t wardriver_cpu_load_poll(void);

// NimBLE host APIs (sniffer observer only; omitted when CONFIG_BT_ENABLED=n).
#if __has_include("nimble/nimble_port.h")
#include "nimble/nimble_port.h"
#include "nimble/nimble_port_freertos.h"
#include "host/ble_hs.h"
#include "host/ble_gap.h"
#include "host/ble_gatt.h"
#include "host/ble_att.h"
#include "host/ble_hs_mbuf.h"
#include "os/os_mbuf.h"
#include "services/gap/ble_svc_gap.h"
#include "services/gatt/ble_svc_gatt.h"
#include "host/ble_store.h"

/** ESP-IDF NimBLE: wire ble_hs_cfg store callbacks (not in a public header in IDF 6.x). */
void ble_store_config_init(void);
#endif

// Wi-Fi Aware NAN-USD (PackLink gateway phone link; requires CONFIG_ESP_WIFI_NAN_USD_ENABLE).
#if __has_include("esp_nan.h")
#include "esp_nan.h"
#endif
