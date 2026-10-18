//! Board pin profile constants (host-testable; no esp-idf).

use wardriver_board::{MARAUDER, XIAO};

#[test]
fn xiao_pin_constants() {
    assert_eq!(XIAO.name, "xiao");
    assert_eq!(XIAO.sd_sck, 8);
    assert_eq!(XIAO.sd_cs, 25);
    assert_eq!(XIAO.gps_uart_tx, 11);
    assert_eq!(XIAO.buzzer_gpio, Some(7));
    assert_eq!(XIAO.oled_i2c_sda, Some(23));
}

#[test]
fn marauder_pin_constants() {
    assert_eq!(MARAUDER.name, "marauder");
    assert_eq!(MARAUDER.sd_sck, 6);
    assert_eq!(MARAUDER.buzzer_gpio, None);
    assert_eq!(MARAUDER.oled_i2c_sda, None);
}

#[cfg(feature = "board-xiao")]
#[test]
fn active_profile_xiao() {
    use wardriver_board::active_profile;
    assert_eq!(active_profile().name, "xiao");
}

#[cfg(feature = "board-marauder")]
#[test]
fn active_profile_marauder() {
    use wardriver_board::active_profile;
    assert_eq!(active_profile().name, "marauder");
}
