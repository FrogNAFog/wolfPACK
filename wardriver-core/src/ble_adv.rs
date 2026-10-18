//! Bluetooth LE advertisement (AD) parsing for wardrive CSV enrichment.
//!
//! Primary reference: WiGLE `BluetoothReceiver.handleLeScanResult`.

use crate::wigle_ble::MAJOR_UNCATEGORIZED;

const AD_TYPE_SHORT_NAME: u8 = 0x08;
const AD_TYPE_COMPLETE_NAME: u8 = 0x09;
const AD_TYPE_CLASS_OF_DEVICE: u8 = 0x0D;
const AD_TYPE_MFG: u8 = 0xFF;

/// Parsed BLE advertisement fields used for WiGLE CSV rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BleAdvFields {
    pub name: String,
    pub company_id: u16,
    pub cod: Option<[u8; 3]>,
}

impl Default for BleAdvFields {
    fn default() -> Self {
        Self {
            name: String::new(),
            company_id: 0,
            cod: None,
        }
    }
}

/// Parse AD structures from raw advertisement bytes.
#[must_use]
pub fn parse_ble_adv(data: &[u8]) -> BleAdvFields {
    let mut out = BleAdvFields::default();
    let mut short_name: Option<&[u8]> = None;
    let mut complete_name: Option<&[u8]> = None;
    let mut i = 0usize;
    while i < data.len() {
        let len = data[i] as usize;
        i += 1;
        if len == 0 {
            break;
        }
        if i + len > data.len() {
            break;
        }
        let ad_type = data[i];
        let payload = &data[i + 1..i + len];
        match ad_type {
            AD_TYPE_COMPLETE_NAME => complete_name = Some(payload),
            AD_TYPE_SHORT_NAME => short_name = Some(payload),
            AD_TYPE_MFG if payload.len() >= 2 => {
                out.company_id = u16::from_le_bytes([payload[0], payload[1]]);
            }
            AD_TYPE_CLASS_OF_DEVICE if payload.len() >= 3 => {
                out.cod = Some([payload[0], payload[1], payload[2]]);
            }
            _ => {}
        }
        i += len;
    }
    let chosen = complete_name.or(short_name).unwrap_or(&[]);
    out.name = trim_ascii_name(chosen);
    out
}

/// WiGLE `MfgrId` decimal string from company ID (omit when zero).
#[must_use]
pub fn ble_mfgr_id_string(company_id: u16) -> String {
    if company_id == 0 {
        String::new()
    } else {
        company_id.to_string()
    }
}

fn trim_ascii_name(src: &[u8]) -> String {
    let mut start = 0usize;
    let mut end = src.len();
    while start < end && src[start].is_ascii_whitespace() {
        start += 1;
    }
    while end > start && src[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    String::from_utf8_lossy(&src[start..end]).into_owned()
}

/// Default frequency code when AD has no Class of Device.
pub const BLE_DEFAULT_FREQUENCY_CODE: u16 = MAJOR_UNCATEGORIZED;
