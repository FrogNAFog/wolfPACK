/*
 * FreeRTOS run-time stats → single 0–100% "non-idle" CPU load for dashboard.
 * Uses uxTaskGetSystemState deltas (~1 Hz caller); see ESP-IDF real_time_stats example.
 */
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#define WDRV_CPU_TASKS_MAX 40

static TaskStatus_t s_prev[WDRV_CPU_TASKS_MAX];
static UBaseType_t s_prev_n;
static uint32_t s_prev_total;
static int s_have_prev;

static int is_idle_task_name(const char *name)
{
    return name != NULL && strncmp(name, "IDLE", 4) == 0;
}

static uint32_t prev_runtime_for_handle(TaskHandle_t h)
{
    for (UBaseType_t j = 0; j < s_prev_n; j++) {
        if (s_prev[j].xHandle == h) {
            return (uint32_t)s_prev[j].ulRunTimeCounter;
        }
    }
    return 0U;
}

uint8_t wardriver_cpu_load_poll(void)
{
    TaskStatus_t cur[WDRV_CPU_TASKS_MAX];
    uint32_t cur_total = 0U;
    UBaseType_t n = uxTaskGetSystemState(cur, WDRV_CPU_TASKS_MAX, &cur_total);

    if (n == 0U) {
        return 0U;
    }

    if (!s_have_prev) {
        memcpy(s_prev, cur, sizeof(TaskStatus_t) * (size_t)n);
        s_prev_n = n;
        s_prev_total = cur_total;
        s_have_prev = 1;
        return 0U;
    }

    uint32_t dt_total = cur_total - s_prev_total;
    if (dt_total == 0U) {
        memcpy(s_prev, cur, sizeof(TaskStatus_t) * (size_t)n);
        s_prev_n = n;
        s_prev_total = cur_total;
        return 0U;
    }

    uint32_t idle_dt = 0U;
    for (UBaseType_t i = 0; i < n; i++) {
        const char *name = cur[i].pcTaskName;
        if (!is_idle_task_name(name)) {
            continue;
        }
        uint32_t c = (uint32_t)cur[i].ulRunTimeCounter;
        uint32_t p = prev_runtime_for_handle(cur[i].xHandle);
        idle_dt += (c >= p) ? (c - p) : c;
    }

    uint32_t busy_dt = (idle_dt < dt_total) ? (dt_total - idle_dt) : 0U;
    uint32_t pct = (busy_dt * 100U) / dt_total;
    if (pct > 100U) {
        pct = 100U;
    }

    memcpy(s_prev, cur, sizeof(TaskStatus_t) * (size_t)n);
    s_prev_n = n;
    s_prev_total = cur_total;

    return (uint8_t)pct;
}
