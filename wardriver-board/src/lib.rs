//! Board pin profiles for supported control-node hardware targets.
//!
//! GPIO numbers only — no esp-idf types or drivers.

/// Pin map for SD (SPI), GPS UART, and optional XIAO expansion peripherals.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardProfile {
    pub name: &'static str,
    pub sd_sck: u8,
    pub sd_mosi: u8,
    pub sd_miso: u8,
    pub sd_cs: u8,
    pub sd_cs_label: &'static str,
    pub gps_uart_tx: u8,
    pub gps_uart_rx: u8,
    /// Passive buzzer GPIO (XIAO expansion base D3); absent on Marauder.
    pub buzzer_gpio: Option<u8>,
    /// On-board OLED I2C SDA (XIAO); absent on Marauder.
    pub oled_i2c_sda: Option<u8>,
    /// On-board OLED I2C SCL (XIAO); absent on Marauder.
    pub oled_i2c_scl: Option<u8>,
}

/// ESP32 Marauder C5 adapter pin map (SD + GPS UART).
pub const MARAUDER: BoardProfile = BoardProfile {
    name: "marauder",
    sd_sck: 6,
    sd_mosi: 7,
    sd_miso: 2,
    sd_cs: 10,
    sd_cs_label: "CS",
    gps_uart_tx: 13,
    gps_uart_rx: 14,
    buzzer_gpio: None,
    oled_i2c_sda: None,
    oled_i2c_scl: None,
};

/// Seeed XIAO ESP32-C5 on expansion base (stacked L76K GNSS, on-board SD, shield buzzer/OLED).
/// Active profile for the current firmware build (`board-xiao` or `board-marauder`).
#[must_use]
pub fn active_profile() -> &'static BoardProfile {
    #[cfg(feature = "board-xiao")]
    {
        return &XIAO;
    }
    #[cfg(feature = "board-marauder")]
    {
        return &MARAUDER;
    }
    #[cfg(not(any(feature = "board-xiao", feature = "board-marauder")))]
    {
        &MARAUDER
    }
}

pub const XIAO: BoardProfile = BoardProfile {
    name: "xiao",
    sd_sck: 8,
    sd_mosi: 10,
    sd_miso: 9,
    sd_cs: 25,
    sd_cs_label: "CS/D2",
    gps_uart_tx: 11,
    gps_uart_rx: 12,
    buzzer_gpio: Some(7),
    oled_i2c_sda: Some(23),
    oled_i2c_scl: Some(24),
};
