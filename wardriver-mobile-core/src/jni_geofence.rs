//! Thin JNI wrappers for the geofence / home-zone domain.
//!
//! The `...Json` wrappers delegate to [`crate::jni_bridge`] via the shared
//! [`crate::jni_envelope`] helper. Symbol names are name-bound by the Android
//! loader and must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JClass, JString};
#[cfg(feature = "jni")]
use jni::sys::jstring;
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::jni_bridge::{
    geofence_monitor_update_json, home_zone_contains_json, home_zone_latch_update_json,
};
use crate::jni_envelope::jstring_envelope;

/// JNI: geofence monitor state-machine update.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_geofenceMonitorUpdateJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, geofence_monitor_update_json)
}

/// JNI: home zone latch update.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_homeZoneLatchUpdateJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, home_zone_latch_update_json)
}

/// JNI: stateless home zone contains test.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_homeZoneContainsJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, home_zone_contains_json)
}
