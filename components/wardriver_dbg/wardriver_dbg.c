#include "esp_rom_sys.h"
#include "esp_system.h"

// This runs during ESP-IDF system initialization, before app_main().
ESP_SYSTEM_INIT_FN(wardriver_dbg_init, ESP_SYSTEM_INIT_STAGE_CORE, 100)
{
    esp_rom_printf("[SYS_INIT] core stage reached\n");
    return ESP_OK;
}

