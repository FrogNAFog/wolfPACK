//! JNI surface for Android (`wardriver-mobile-core` built with `--features jni`).
//!
//! **How to read this file:** it is the *barrel* for the JNI entry points. The 46
//! `Java_net_wardriver_mobile_NativeCore_*` fns live in the `jni_*` domain modules
//! (`jni_packlink`, `jni_enrich_wrap`, `jni_csv`, `jni_gps_timeline`, `jni_geofence`,
//! `jni_ingest`), declared in `lib.rs`; this file keeps only the shared binary-array
//! helpers they reuse plus the host-testable [`geofence_update`].
//!
//! **Symbol names are name-bound, not compiler-checked.** The Android loader resolves
//! each entry point by the exact string `Java_<kotlin-package>_NativeCore_<fn>`, and the
//! Kotlin package is `net.wardriver.mobile` (one `d` in `wardriver`). Rust compiles any
//! symbol name, so a typo in the prefix (e.g. `warddriver`) builds green and only fails
//! at runtime with `UnsatisfiedLinkError`. To verify a build, `nm -D` the `.so` and
//! confirm every symbol starts with `Java_net_wardriver_mobile_NativeCore_` — do not
//! rely on the compiler resolving the names.

#[cfg(feature = "jni")]
use jni::objects::{
    JDoubleArray, JFloatArray, JIntArray, JLongArray, JObject, JObjectArray, JString,
};
#[cfg(feature = "jni")]
use jni::sys::{jboolean, jobjectArray};
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::geofence::GeofenceMonitor;
#[cfg(feature = "jni")]
use crate::jni_arrays::ClusterArraysResult;

/// Host-testable geofence monitor wrapper (no JNI).
#[must_use]
pub fn geofence_update(
    monitor: &mut GeofenceMonitor,
    lat: f64,
    lon: f64,
) -> crate::geofence::GeofenceEvent {
    monitor.update(lat, lon)
}

// --- Shared binary-JNI array helpers (used by the `jni_*` domain modules) ---

#[cfg(feature = "jni")]
pub fn read_double_array(env: &mut JNIEnv, arr: &JDoubleArray) -> Vec<f64> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![0.0; len];
    let _ = env.get_double_array_region(arr, 0, &mut out);
    out
}

#[cfg(feature = "jni")]
pub fn read_long_array(env: &mut JNIEnv, arr: &JLongArray) -> Vec<i64> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![0i64; len];
    let _ = env.get_long_array_region(arr, 0, &mut out);
    out
}

#[cfg(feature = "jni")]
pub fn read_float_array(env: &mut JNIEnv, arr: &JFloatArray) -> Vec<f32> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![0.0f32; len];
    let _ = env.get_float_array_region(arr, 0, &mut out);
    out
}

#[cfg(feature = "jni")]
pub fn read_int_array(env: &mut JNIEnv, arr: &JIntArray) -> Vec<i32> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut out = vec![0i32; len];
    let _ = env.get_int_array_region(arr, 0, &mut out);
    out
}

#[cfg(feature = "jni")]
pub fn read_string_array(env: &mut JNIEnv, arr: &JObjectArray) -> Vec<String> {
    let len = env.get_array_length(arr).unwrap_or(0).max(0) as usize;
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let obj = env.get_object_array_element(arr, i as i32).ok();
        if let Some(jobj) = obj {
            if let Ok(s) = env.get_string(&JString::from(jobj)) {
                out.push(s.into());
            } else {
                out.push(String::new());
            }
        } else {
            out.push(String::new());
        }
    }
    out
}

#[cfg(feature = "jni")]
pub fn bool_to_jboolean(value: bool) -> jboolean {
    if value {
        1
    } else {
        0
    }
}

#[cfg(feature = "jni")]
pub fn new_double_array<'local>(env: &mut JNIEnv<'local>, values: &[f64]) -> JDoubleArray<'local> {
    let arr = env
        .new_double_array(values.len() as i32)
        .expect("double array");
    if !values.is_empty() {
        env.set_double_array_region(&arr, 0, values)
            .expect("set double array");
    }
    arr
}

#[cfg(feature = "jni")]
pub fn new_int_array<'local>(env: &mut JNIEnv<'local>, values: &[i32]) -> JIntArray<'local> {
    let arr = env.new_int_array(values.len() as i32).expect("int array");
    if !values.is_empty() {
        env.set_int_array_region(&arr, 0, values)
            .expect("set int array");
    }
    arr
}

#[cfg(feature = "jni")]
pub fn cluster_to_object_array<'local>(
    env: &mut JNIEnv<'local>,
    result: ClusterArraysResult,
) -> jobjectArray {
    let obj_cls = env.find_class("java/lang/Object").expect("Object class");
    let string_cls = env.find_class("java/lang/String").expect("String");
    let out = env
        .new_object_array(6, obj_cls, JObject::null())
        .expect("object array");
    let lats = new_double_array(env, &result.lats);
    let lons = new_double_array(env, &result.lons);
    let counts = new_int_array(env, &result.counts);
    let rssi = new_int_array(env, &result.best_rssi);
    let ssids = env
        .new_object_array(result.label_ssids.len() as i32, string_cls, JObject::null())
        .expect("ssid array");
    for (i, s) in result.label_ssids.iter().enumerate() {
        let jstr = env.new_string(s).expect("ssid string");
        env.set_object_array_element(&ssids, i as i32, jstr)
            .expect("set ssid");
    }
    let members_cls = env.find_class("java/lang/Object").expect("Object");
    let members_outer = env
        .new_object_array(
            result.member_indices.len() as i32,
            members_cls,
            JObject::null(),
        )
        .expect("members outer");
    for (i, indices) in result.member_indices.iter().enumerate() {
        let inner = new_int_array(env, indices);
        env.set_object_array_element(&members_outer, i as i32, inner)
            .expect("set member indices");
    }
    env.set_object_array_element(&out, 0, lats)
        .expect("set lats");
    env.set_object_array_element(&out, 1, lons)
        .expect("set lons");
    env.set_object_array_element(&out, 2, counts)
        .expect("set counts");
    env.set_object_array_element(&out, 3, rssi)
        .expect("set rssi");
    env.set_object_array_element(&out, 4, ssids)
        .expect("set ssids");
    env.set_object_array_element(&out, 5, members_outer)
        .expect("set members");
    out.into_raw()
}
