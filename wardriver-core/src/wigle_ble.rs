//! WiGLE Android Bluetooth CSV helpers (`DEVICE_TYPE_LEGEND` + CoD AD `0x0D`).
//!
//! Primary reference: WiGLE `BluetoothReceiver.handleLeScanResult`.

/// `BluetoothClass.Device.Major.UNCATEGORIZED` (0x1F00).
pub const MAJOR_UNCATEGORIZED: u16 = 0x1F00;

/// WiGLE Android `DEVICE_TYPE_LEGEND` label for a Bluetooth device-class code.
///
/// Returns an empty string when the code is absent from the frozen legend (Android
/// leaves AuthMode blank on export).
#[must_use]
pub fn device_type_label(device_class: u16) -> &'static str {
    DEVICE_TYPE_LEGEND
        .iter()
        .find(|(k, _)| *k == device_class)
        .map(|(_, v)| *v)
        .unwrap_or("")
}

/// Frozen WiGLE Android `DEVICE_TYPE_LEGEND` (AuthMode column; no `[LE]` suffix).
const DEVICE_TYPE_LEGEND: &[(u16, &str)] = &[
    (0x0000, "Misc"),
    (0x0100, "Computer"),
    (0x0104, "Desktop"),
    (0x0108, "Server"),
    (0x010C, "Laptop"),
    (0x0110, "PDA"),
    (0x0114, "Palm"),
    (0x0118, "Wearable Computer"),
    (0x0200, "Phone"),
    (0x0204, "Cellphone"),
    (0x0208, "Cordless Phone"),
    (0x020C, "Smartphone"),
    (0x0210, "Modem/GW"),
    (0x0214, "ISDN"),
    (0x0400, "A/V"),
    (0x0408, "Handsfree"),
    (0x0410, "Mic"),
    (0x0414, "Speaker"),
    (0x0418, "Headphones"),
    (0x041C, "Portable Audio"),
    (0x0420, "Car Audio"),
    (0x0424, "Settop"),
    (0x0428, "HiFi"),
    (0x042C, "VCR"),
    (0x0430, "Camera"),
    (0x0434, "Camcorder"),
    (0x0438, "Monitor"),
    (0x043C, "Display/Speaker"),
    (0x0440, "Videoconf"),
    (0x0448, "AV Toy"),
    (0x0500, "Keyboard !p"),
    (0x0540, "Keyboard"),
    (0x0580, "Pointer"),
    (0x05C0, "Keyboard+p"),
    (0x0700, "Wearable"),
    (0x0704, "Watch"),
    (0x0708, "Pager"),
    (0x070C, "Jacket"),
    (0x0710, "Helmet"),
    (0x0714, "Glasses"),
    (0x0800, "Toy"),
    (0x0804, "Robot"),
    (0x0808, "Vehicle"),
    (0x080C, "Doll"),
    (0x0810, "Controller"),
    (0x0814, "Game"),
    (0x0900, "Health"),
    (0x0904, "Blood Pressure"),
    (0x0908, "Thermometer"),
    (0x090C, "Scale"),
    (0x0910, "Glucose"),
    (0x0914, "PulseOxy"),
    (0x0918, "Pulse"),
    (0x091C, "Health Display"),
    (0x1F00, "Uncategorized"),
];

/// Parse Class of Device AD bytes (little-endian 24-bit) per Android `handleLeScanResult`.
///
/// Returns `(capabilities label, device-type frequency code)`.
#[must_use]
pub fn ble_csv_fields_from_cod(b0: u8, b1: u8, b2: u8) -> (String, u16) {
    let cod = u32::from_le_bytes([b0, b1, b2, 0]);
    let device_class = (cod & 0x1FFC) as u16;
    let major_class = (cod & 0x1F00) as u16;

    let device_type = if device_class == 0 || device_class == MAJOR_UNCATEGORIZED {
        major_class
    } else {
        device_class
    };

    let capabilities = device_type_label(device_type).to_string();
    (capabilities, device_type)
}

/// Default CSV fields when no CoD AD is present (Android `bluetoothClass == null`).
#[must_use]
pub fn ble_csv_fields_no_cod() -> (String, u16) {
    (String::new(), MAJOR_UNCATEGORIZED)
}
