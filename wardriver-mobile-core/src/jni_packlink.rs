//! Thin JNI wrappers for the PackLink domain (pack/unpack + probe observation bulk).
//!
//! These delegate to [`crate::jni_enrich`] (the real pack/unpack logic) and
//! [`wardriver_packlink`]. Symbol names are name-bound by the Android loader and
//! must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JByteArray, JClass, JString};
#[cfg(feature = "jni")]
use jni::sys::{jlong, jstring};
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::jni_enrich::{
    pack_capture_cmd_json, pack_channel_plan_sync_json, pack_policy_sync_json, pack_time_sync_json,
    probe_observation_bulk_json, unpack_capture_status_json, unpack_channel_plan_sync_ack_json,
    unpack_event_notify_json, unpack_observation_bulk_json, unpack_packlink_frame_json,
    unpack_policy_sync_ack_json, unpack_sniffer_status_json, unpack_spool_replay_json,
    unpack_time_sync_json,
};
use crate::jni_envelope::jstring_envelope;

/// JNI: PackLink magic string for BLE frame validation.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_packlinkMagic(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let s = std::str::from_utf8(&wardriver_packlink::MAGIC).unwrap_or("PLNK");
    env.new_string(s).expect("jni string").into_raw()
}

/// JNI: unpack a PackLink wire frame; returns JSON (`msg_type`, `flags`, `seq`, `payload_hex`).
///
/// MVP returns a single JSON string instead of split msgType + payload calls.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackPackLinkFrameJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_packlink_frame_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: unpack ObservationBulk PackLink frame; returns JSON with AP list.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackObservationBulkJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_observation_bulk_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: pack PolicySyncV1 from JSON; returns JSON with `wire_hex`.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_packPolicySyncJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, pack_policy_sync_json)
}

/// JNI: unpack SnifferStatus PackLink frame; returns JSON with `rows` array.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackSnifferStatusJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_sniffer_status_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: pack ChannelPlanSyncV1 from JSON; returns JSON with `wire_hex`.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_packChannelPlanSyncJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, pack_channel_plan_sync_json)
}

/// JNI: unpack ChannelPlanSyncAck PackLink frame.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackChannelPlanSyncAckJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_channel_plan_sync_ack_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: unpack PolicySyncAck PackLink frame.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackPolicySyncAckJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_policy_sync_ack_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: pack CaptureCmdV1 from JSON; returns JSON with `wire_hex`.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_packCaptureCmdJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, pack_capture_cmd_json)
}

/// JNI: unpack CaptureStatus PackLink frame; returns JSON progress fields.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackCaptureStatusJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_capture_status_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: unpack SpoolReplay PackLink frame; returns JSON chunk metadata + payload hex.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackSpoolReplayJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_spool_replay_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: pack TimeSyncV1 from JSON; returns JSON with `wire_hex`.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_packTimeSyncJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, pack_time_sync_json)
}

/// JNI: unpack TimeSync PackLink frame.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackTimeSyncJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_time_sync_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: unpack EventNotify PackLink frame with nested wolfPACK alert detail.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_unpackEventNotifyJson(
    env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("packlink wire bytes");
    let json = unpack_event_notify_json(&bytes);
    env.new_string(json).expect("jni string").into_raw()
}

/// JNI: unpack ProbeObservationBulk PackLink frame; returns JSON with `observations` array.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_probeObservationBulkJson(
    mut env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
    session_id: jlong,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("wire bytes");
    let out = probe_observation_bulk_json(&bytes, session_id as i64);
    env.new_string(out).expect("jni string").into_raw()
}
