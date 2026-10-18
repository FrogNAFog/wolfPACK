//! XIAO user LED (GPIO27) ack flashes for gateway fleet unbind.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::Result;
use esp_idf_hal::gpio::{Output, OutputPin, PinDriver};
use wardriver_shared::spiram_thread;
use wardriver_shared::task_wdt;

/// Seeed XIAO ESP32-C5 yellow user LED (`LED_BUILTIN`).
pub const XIAO_USER_LED_GPIO: u8 = 27;

const WORKER_STACK_BYTES: usize = 6_144;

/// XIAO user LED is active-low (GPIO low = lit).
const LED_ACTIVE_HIGH: bool = false;

static FLASH_REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct GatewayLedHandle {
    _worker: JoinHandle<()>,
}

impl GatewayLedHandle {
    pub fn spawn(pin: impl OutputPin + 'static) -> Result<Self> {
        let worker = spiram_thread::spawn_named("gateway-led", WORKER_STACK_BYTES, move || {
            let Ok(mut pin) = PinDriver::output(pin) else {
                return;
            };
            set_lit(&mut pin, false);
            gateway_led_worker(&mut pin);
        })?;
        Ok(Self { _worker: worker })
    }
}

/// Three short flashes after a successful fleet unbind.
pub fn request_success_flash() {
    FLASH_REQUESTED.store(true, Ordering::Release);
}

fn gateway_led_worker(pin: &mut PinDriver<'static, Output>) {
    loop {
        task_wdt::reset_if_subscribed();
        if FLASH_REQUESTED.swap(false, Ordering::AcqRel) {
            for _ in 0..3 {
                set_lit(pin, true);
                task_wdt::sleep_feeding_wdt(Duration::from_millis(120));
                set_lit(pin, false);
                task_wdt::sleep_feeding_wdt(Duration::from_millis(120));
            }
        }
        task_wdt::sleep_feeding_wdt(Duration::from_millis(40));
    }
}

fn set_lit(pin: &mut PinDriver<'static, Output>, on: bool) {
    let lit = if LED_ACTIVE_HIGH { on } else { !on };
    if lit {
        let _ = pin.set_low();
    } else {
        let _ = pin.set_high();
    }
}
