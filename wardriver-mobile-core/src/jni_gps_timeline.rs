//! Thin JNI wrappers for the GPS timeline domain.
//!
//! Handle- and scalar-based wrappers delegate to [`crate::jni_arrays`] /
//! [`crate::gps_timeline`] using the shared array helpers in [`crate::jni`];
//! the `...Json` wrapper uses the shared [`crate::jni_envelope`] helper.
//! Symbol names are name-bound by the Android loader and must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JClass, JDoubleArray, JFloatArray, JLongArray, JString};
#[cfg(feature = "jni")]
use jni::sys::{jboolean, jdouble, jfloat, jlong, jstring};
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::jni::{bool_to_jboolean, read_double_array, read_float_array, read_long_array};
use crate::jni_arrays::{
    gps_timeline_append, gps_timeline_create, gps_timeline_destroy, gps_timeline_merge_route,
    gps_timeline_prune, gps_timeline_to_hist_json,
};
use crate::jni_bridge::gps_timeline_op_json;
use crate::jni_envelope::jstring_envelope;

/// JNI: allocate native GPS timeline handle.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineCreate(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    gps_timeline_create()
}

/// JNI: release native GPS timeline handle.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    gps_timeline_destroy(handle);
}

/// JNI: append one GPS fix to a native timeline.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineAppend(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    unix_ms: jlong,
    lat: jdouble,
    lon: jdouble,
    accuracy_m: jfloat,
    altitude_m: jdouble,
    has_altitude: jboolean,
    now_ms: jlong,
) -> jboolean {
    let alt = if has_altitude != 0 {
        Some(altitude_m)
    } else {
        None
    };
    bool_to_jboolean(gps_timeline_append(
        handle, unix_ms, lat, lon, accuracy_m, alt, now_ms,
    ))
}

/// JNI: prune timeline samples older than retention.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelinePrune(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    now_ms: jlong,
) {
    gps_timeline_prune(handle, now_ms);
}

/// JNI: merge route points into a native timeline.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineMergeRoute(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    unix_ms: JLongArray,
    lats: JDoubleArray,
    lons: JDoubleArray,
    accuracy_m: JFloatArray,
    now_ms: jlong,
) {
    let u = read_long_array(&mut env, &unix_ms);
    let la = read_double_array(&mut env, &lats);
    let lo = read_double_array(&mut env, &lons);
    let acc = read_float_array(&mut env, &accuracy_m);
    gps_timeline_merge_route(handle, &u, &la, &lo, &acc, now_ms);
}

/// JNI: JSON snapshot of timeline for ingest `gps_hist`.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineToHistJson(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let json = gps_timeline_to_hist_json(handle);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: GPS fix accuracy usable for CSV gate.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineIsUsable(
    _env: JNIEnv,
    _class: JClass,
    accuracy_m: jfloat,
) -> jboolean {
    bool_to_jboolean(crate::gps_timeline::GpsTimeline::is_usable(accuracy_m))
}

/// JNI: fix timestamp within allowed skew.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineIsValidFixTime(
    _env: JNIEnv,
    _class: JClass,
    unix_ms: jlong,
    now_ms: jlong,
) -> jboolean {
    bool_to_jboolean(crate::gps_timeline::GpsTimeline::is_valid_fix_time(
        unix_ms, now_ms,
    ))
}

/// JNI: replay bulk older than retention window.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineIsStaleReplay(
    _env: JNIEnv,
    _class: JClass,
    hdr_scan_end_unix_ms: jlong,
    now_ms: jlong,
) -> jboolean {
    bool_to_jboolean(crate::gps_timeline::GpsTimeline::is_stale_replay(
        hdr_scan_end_unix_ms,
        now_ms,
    ))
}

/// JNI: GPS timeline operations (append, prune, merge_route, constants).
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_gpsTimelineOpJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, gps_timeline_op_json)
}
