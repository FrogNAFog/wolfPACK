# Ensures `esp_driver_tsens` is linked so SoC temperature FFI resolves.
# `wardriver_cpu_stats.c`: FreeRTOS run-time stats → dashboard CPU % (see `wardriver_cpu_load_poll` in bindings).
idf_component_register(
    SRCS "wardriver_tsens.c" "wardriver_cpu_stats.c"
    REQUIRES esp_driver_tsens freertos
    INCLUDE_DIRS "."
)
