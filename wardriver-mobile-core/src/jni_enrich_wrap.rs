//! Thin JNI wrappers for the enrichment domain + map/coverage/route array helpers.
//!
//! The `...Json` wrappers delegate to [`crate::jni_enrich`] / [`crate::jni_bridge`];
//! the array wrappers delegate to [`crate::jni_arrays`] using the shared binary
//! array helpers in [`crate::jni`]. Symbol names are name-bound by the Android
//! loader and must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JClass, JDoubleArray, JIntArray, JObjectArray, JString};
#[cfg(feature = "jni")]
use jni::sys::{jdouble, jint, jobjectArray, jstring};
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::enrichment::REVISIT_RADIUS_M;
use crate::jni::{
    cluster_to_object_array, new_double_array, new_int_array, read_double_array, read_int_array,
    read_string_array,
};
use crate::jni_arrays::{
    cluster_map_markers_arrays, coverage_grid_arrays, default_simplify_max_points,
    simplify_route_arrays,
};
use crate::jni_bridge::sniffer_display_json;
use crate::jni_enrich::{
    enrich_ble_ap_json, enrich_cell_json, enrich_wifi_ap_json, enrichment_session_close_json,
    enrichment_session_open_json,
};
use crate::jni_envelope::jstring_envelope;

/// JNI: return revisit radius in meters (50.0).
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_revisitRadiusM(
    _env: JNIEnv,
    _class: JClass,
) -> jdouble {
    REVISIT_RADIUS_M
}

/// JNI: enrich one WiFi AP from a JSON request string; returns JSON outcome.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_enrichWifiApJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, enrich_wifi_ap_json)
}

/// JNI: enrich one BLE device from a JSON request string; returns JSON outcome.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_enrichBleApJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, enrich_ble_ap_json)
}

/// JNI: enrich one phone-sourced cell tower from a JSON request string.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_enrichCellJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, enrich_cell_json)
}

/// JNI: open wardrive enrichment session (empty per-run MacTable).
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_enrichmentSessionOpenJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, enrichment_session_open_json)
}

/// JNI: close wardrive enrichment session.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_enrichmentSessionCloseJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, enrichment_session_close_json)
}

/// JNI: sniffer display helpers (quiet_stale, last_seen_ago, etc.).
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_snifferDisplayJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, sniffer_display_json)
}

/// JNI: Douglas-Peucker simplification; returns interleaved [lat, lon, ...].
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_simplifyRouteArrays<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    lats: JDoubleArray<'local>,
    lons: JDoubleArray<'local>,
    max_points: jint,
) -> JDoubleArray<'local> {
    let la = read_double_array(&mut env, &lats);
    let lo = read_double_array(&mut env, &lons);
    let max = if max_points > 0 {
        max_points as usize
    } else {
        default_simplify_max_points()
    };
    let (out_lats, out_lons) = simplify_route_arrays(&la, &lo, max);
    let mut interleaved = Vec::with_capacity(out_lats.len() * 2);
    for i in 0..out_lats.len() {
        interleaved.push(out_lats[i]);
        interleaved.push(out_lons[i]);
    }
    new_double_array(&mut env, &interleaved)
}

/// JNI: cluster markers; returns Object[6] {lats, lons, counts, bestRssi, ssids, memberIndices}.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_clusterMapMarkersArrays(
    mut env: JNIEnv,
    _class: JClass,
    lats: JDoubleArray,
    lons: JDoubleArray,
    rssis: JIntArray,
    ssids: JObjectArray,
    bssids: JObjectArray,
    signal_kinds: JObjectArray,
    zoom: jdouble,
    cluster_zoom_threshold: jdouble,
    coord_decimals: jint,
) -> jobjectArray {
    let la = read_double_array(&mut env, &lats);
    let lo = read_double_array(&mut env, &lons);
    let rssi = read_int_array(&mut env, &rssis);
    let ssid_strs = read_string_array(&mut env, &ssids);
    let bssid_strs = read_string_array(&mut env, &bssids);
    let kind_strs = read_string_array(&mut env, &signal_kinds);
    let result = cluster_map_markers_arrays(
        &la,
        &lo,
        &rssi,
        &ssid_strs,
        &bssid_strs,
        &kind_strs,
        zoom,
        cluster_zoom_threshold,
        if coord_decimals > 0 {
            coord_decimals as u32
        } else {
            crate::map_geometry::COORD_DECIMALS_DEFAULT
        },
    );
    cluster_to_object_array(&mut env, result)
}

/// JNI: coverage grid; returns Object[3] {fraction[1], unvisitedRows, unvisitedCols}.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_coverageGridArrays(
    mut env: JNIEnv,
    _class: JClass,
    lat_min: jdouble,
    lat_max: jdouble,
    lon_min: jdouble,
    lon_max: jdouble,
    rows: jint,
    cols: jint,
    route_lats: JDoubleArray,
    route_lons: JDoubleArray,
) -> jobjectArray {
    use jni::objects::JObject;
    let la = read_double_array(&mut env, &route_lats);
    let lo = read_double_array(&mut env, &route_lons);
    let result = coverage_grid_arrays(
        lat_min,
        lat_max,
        lon_min,
        lon_max,
        rows.max(1) as u32,
        cols.max(1) as u32,
        &la,
        &lo,
    );
    let obj_cls = env.find_class("java/lang/Object").expect("Object");
    let out = env
        .new_object_array(3, obj_cls, JObject::null())
        .expect("object array");
    let fraction = new_double_array(&mut env, &[result.fraction]);
    let urows = new_int_array(&mut env, &result.unvisited_rows);
    let ucols = new_int_array(&mut env, &result.unvisited_cols);
    env.set_object_array_element(&out, 0, fraction)
        .expect("fraction");
    env.set_object_array_element(&out, 1, urows).expect("rows");
    env.set_object_array_element(&out, 2, ucols).expect("cols");
    out.into_raw()
}
