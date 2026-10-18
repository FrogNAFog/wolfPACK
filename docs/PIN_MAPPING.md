### Pin mapping (XIAO ESP32-C5 + Seeed Expansion Board Base)

Build with **`board-xiao`** when the XIAO is mounted on the [Seeed XIAO Expansion Board](https://wiki.seeedstudio.com/Seeeduino-XIAO-Expansion-board/) style base. Pin names follow the shield / XIAO **D** labels.

**GNSS (stacked Seeed XIAO L76K):**

- Stacked **L76K** on **D6/D7** (**GPIO11** TX → GNSS RX, **GPIO12** RX ← GNSS TX), **9600** baud, **1 Hz**. Firmware sends PCAS04/03/02 at boot (GPS+BeiDou+GLONASS, GGA+RMC NMEA only).
- **Mechanical / RF:** confirm header stack height lets the GNSS module clear the expansion board before relying on GPS in the field.

**microSD (on expansion base, SPI / SDSPI):**

- **SCK** → **D8 (GPIO8)**
- **MOSI** → **D10 (GPIO10)**
- **MISO** → **D9 (GPIO9)**
- **CS** → **D2 (GPIO25)**

**Buzzer (on shield):** passive buzzer on expansion **“A3”** net → **D3 (GPIO7)**. Driven with **LEDC PWM** (Seeed `analogWrite` / `tone()` style). Distinct short patterns for boot, upload start/done, GPS fix, Flock, and Axon (see below). A separate low tone still plays on SD mount failure.

**Onboard OLED (SSD1306 128×64):** **D4 / D5** (**GPIO23** SDA, **GPIO24** SCL), I2C **100 kHz** at boot (address **0x3C**, fallback **0x3D**). Shows the same event labels as the buzzer for at least **4 seconds** each. Configure which events beep on **`/alerts`** (OLED text is always shown when init succeeds). 

### Pin mapping (Marauder C5 adapter)

- **GPS (UART)**
  - **GPS TX → ESP32 GPIO14** (ESP32 **RX=GPIO14**)
  - **GPS RX → ESP32 GPIO13** (ESP32 **TX=GPIO13**)
  - Baud: **9600** default
- **microSD (SPI / FATFS)**
  - **MISO=GPIO2**
  - **MOSI=GPIO7**
  - **SCK=GPIO6**
  - **CS=GPIO10**
- **Status LED (WS2812)**
  - **GPIO27** (C5-DevKitC-1 on-board RGB), RMT channel 0

