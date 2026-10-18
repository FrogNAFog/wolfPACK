//! Extract OpenDroneID payloads from BLE advertisement data.

const AD_TYPE_SERVICE_DATA_16: u8 = 0x16;
const AD_TYPE_MFG: u8 = 0xFF;

const ASTM_SERVICE_UUID: [u8; 2] = [0xFA, 0xFF];
const LEGACY_MFG_COMPANY_ID: [u8; 2] = [0x02, 0x00];
const ODID_APP_CODE: u8 = 0x0D;

/// Locate an OpenDroneID payload inside raw BLE AD bytes.
///
/// Supports:
/// - Service Data (`0x16`) + UUID `0xFFFA` + app code `0x0D`
/// - Legacy manufacturer data (`0xFF`) + company ID `0x0002` + app code `0x0D`
#[must_use]
pub fn extract_ble_remote_id_payload(adv: &[u8]) -> Option<&[u8]> {
    let mut i = 0usize;
    while i < adv.len() {
        let len = adv[i] as usize;
        i += 1;
        if len == 0 {
            break;
        }
        if i + len > adv.len() {
            break;
        }
        let ad_type = adv[i];
        let payload = &adv[i + 1..i + len];
        match ad_type {
            AD_TYPE_SERVICE_DATA_16 if payload.len() >= 3 => {
                if payload[..2] == ASTM_SERVICE_UUID && payload[2] == ODID_APP_CODE {
                    return payload.get(3..);
                }
            }
            AD_TYPE_MFG if payload.len() >= 3 => {
                if payload[..2] == LEGACY_MFG_COMPANY_ID && payload[2] == ODID_APP_CODE {
                    return payload.get(3..);
                }
            }
            _ => {}
        }
        i += len;
    }
    None
}
