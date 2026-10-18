//! Onboard SSD1306 128×64 OLED on Seeed expansion base (I2C: **D4/GPIO23 SDA**, **D5/GPIO24 SCL**).

use anyhow::{Context, Result};
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Alignment, Text},
};
use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::gpio::{InputPin, OutputPin};
use esp_idf_hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_hal::units::Hertz;
use log::{info, warn};
use ssd1306::{mode::BufferedGraphicsMode, prelude::*, I2CDisplayInterface, Ssd1306};

const OLED_ADDR_PRIMARY: u8 = 0x3C;
const OLED_ADDR_ALT: u8 = 0x3D;

pub struct Display {
    inner: Ssd1306<
        display_interface_i2c::I2CInterface<I2cDriver<'static>>,
        DisplaySize128x64,
        BufferedGraphicsMode<DisplaySize128x64>,
    >,
}

impl Display {
    fn init_with_interface(i2c: I2cDriver<'static>, addr: u8, baud_hz: u32) -> Result<Self> {
        let interface = if addr == OLED_ADDR_ALT {
            I2CDisplayInterface::new_alternate_address(i2c)
        } else {
            I2CDisplayInterface::new(i2c)
        };
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display
            .init()
            .map_err(|e| anyhow::anyhow!("SSD1306 init @0x{addr:02x} {baud_hz}Hz: {e:?}"))?;
        display
            .clear(BinaryColor::Off)
            .map_err(|e| anyhow::anyhow!("SSD1306 clear: {e:?}"))?;
        display
            .flush()
            .map_err(|e| anyhow::anyhow!("SSD1306 flush: {e:?}"))?;
        info!("display: SSD1306 OK @0x{addr:02x} {baud_hz}Hz");
        Ok(Self { inner: display })
    }

    /// Draw title + optional subtitle (does not sleep).
    pub fn show_message(&mut self, title: &str, subtitle: Option<&str>) -> Result<()> {
        self.inner
            .clear(BinaryColor::Off)
            .map_err(|e| anyhow::anyhow!("clear: {e:?}"))?;
        let title_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
        Text::with_alignment(title, Point::new(0, 18), title_style, Alignment::Left)
            .draw(&mut self.inner)
            .map_err(|e| anyhow::anyhow!("title draw: {e:?}"))?;
        if let Some(sub) = subtitle {
            let sub_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
            Text::with_alignment(sub, Point::new(0, 40), sub_style, Alignment::Left)
                .draw(&mut self.inner)
                .map_err(|e| anyhow::anyhow!("subtitle draw: {e:?}"))?;
        }
        self.inner
            .flush()
            .map_err(|e| anyhow::anyhow!("flush: {e:?}"))?;
        Ok(())
    }
}

/// SSD1306 ACKs a command-mode write; empty writes often NACK on clones.
fn probe_addr(i2c: &mut I2cDriver<'_>, addr: u8) -> bool {
    i2c.write(addr, &[0x00], 1_000).is_ok()
}

fn try_open_i2c<I2C: I2c + 'static>(
    i2c: I2C,
    sda: impl InputPin + OutputPin + 'static,
    scl: impl InputPin + OutputPin + 'static,
    baud_hz: u32,
) -> Result<I2cDriver<'static>> {
    let config = I2cConfig::new().baudrate(Hertz(baud_hz));
    I2cDriver::new(i2c, sda, scl, &config).with_context(|| format!("I2C open {baud_hz}Hz"))
}

fn pick_oled_addr(bus: &mut I2cDriver<'_>) -> u8 {
    let at_3c = probe_addr(bus, OLED_ADDR_PRIMARY);
    let at_3d = probe_addr(bus, OLED_ADDR_ALT);
    match (at_3c, at_3d) {
        (true, true) => {
            info!("display: i2c probe found 0x3c, 0x3d");
            OLED_ADDR_PRIMARY
        }
        (true, false) => {
            info!("display: i2c probe found 0x3c");
            OLED_ADDR_PRIMARY
        }
        (false, true) => {
            info!("display: i2c probe found 0x3d");
            OLED_ADDR_ALT
        }
        (false, false) => {
            warn!("display: i2c probe none (trying init @0x3c)");
            OLED_ADDR_PRIMARY
        }
    }
}

fn try_init_from_bus(bus: I2cDriver<'static>, baud_hz: u32) -> Option<Display> {
    let mut bus = bus;
    let addr = pick_oled_addr(&mut bus);
    match Display::init_with_interface(bus, addr, baud_hz) {
        Ok(d) => Some(d),
        Err(e) => {
            warn!("display: {e:#}");
            None
        }
    }
}

/// Init onboard OLED after SD mount (Seeed example: D4/D5, 100 kHz).
pub fn try_init_board<I2C: I2c + 'static>(
    i2c: I2C,
    sda: impl InputPin + OutputPin + 'static,
    scl: impl InputPin + OutputPin + 'static,
) -> Option<Display> {
    FreeRtos::delay_ms(100);
    let bus = try_open_i2c(i2c, sda, scl, 100_000)
        .inspect_err(|e| warn!("display: I2C open failed @100kHz: {e:#}"))
        .ok()?;
    try_init_from_bus(bus, 100_000)
}

/// Board-aware display bring-up: initializes the SSD1306 only when the active board
/// profile exposes on-board OLED I2C pins (absent on boards without an OLED).
///
/// The `board-xiao` pin decision lives here — callers invoke this unconditionally and the
/// adaptor returns `None` on boards that have no display, so no `board-xiao` cfg is needed
/// at the call site. Boards with an OLED that fails to init log a warning here; boards with
/// no OLED return silently so non-display builds produce no extra output.
pub fn try_init_for_board(i2c: esp_idf_hal::i2c::I2C0<'static>) -> Option<Display> {
    use esp_idf_hal::gpio::AnyIOPin;

    let bp = wardriver_board::active_profile();
    let (Some(sda_num), Some(scl_num)) = (bp.oled_i2c_sda, bp.oled_i2c_scl) else {
        return None;
    };
    // SAFETY: the OLED I2C pins are exclusive to this bring-up; the bootloader left them unconfigured.
    let sda = unsafe { AnyIOPin::steal(sda_num) };
    let scl = unsafe { AnyIOPin::steal(scl_num) };
    match try_init_board(i2c, sda, scl) {
        Some(display) => Some(display),
        None => {
            warn!("display: onboard OLED unavailable (buzzer alerts still active)");
            None
        }
    }
}
