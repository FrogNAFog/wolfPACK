//! Thin JNI wrappers for the WiGLE CSV domain.
//!
//! Each wrapper parses the inbound `JString` request and delegates to
//! [`crate::jni_enrich`] / [`crate::jni_bridge`] via the shared
//! [`crate::jni_envelope`] helper. Symbol names are name-bound by the Android
//! loader and must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JClass, JString};
#[cfg(feature = "jni")]
use jni::sys::jstring;
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::jni_bridge::format_wigle_headers_json;
use crate::jni_enrich::{
    format_ble_csv_line_json, format_cell_csv_line_json, format_wifi_csv_line_json,
};
use crate::jni_envelope::jstring_envelope;

/// JNI: format one WiGLE WiFi CSV data line from observation JSON.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_formatWifiCsvLineJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, format_wifi_csv_line_json)
}

/// JNI: format one WiGLE BLE CSV data line from observation JSON.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_formatBleCsvLineJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, format_ble_csv_line_json)
}

/// JNI: format one WiGLE cell CSV data line from observation JSON.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_formatCellCsvLineJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, format_cell_csv_line_json)
}

/// JNI: WiGLE CSV pre-header + column header.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_formatWigleHeadersJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, format_wigle_headers_json)
}
