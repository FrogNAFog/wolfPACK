//! Thin JNI wrappers for the bulk-ingest domain.
//!
//! `ingestCellsJson` uses the shared [`crate::jni_envelope`] helper;
//! `ingestObservationBulkJson` parses wire bytes plus a `JString` context.
//! Symbol names are name-bound by the Android loader and must stay byte-identical.

#[cfg(feature = "jni")]
use jni::objects::{JByteArray, JClass, JString};
#[cfg(feature = "jni")]
use jni::sys::jstring;
#[cfg(feature = "jni")]
use jni::JNIEnv;

use crate::jni_bridge::{ingest_cells_json, ingest_observation_bulk_json};
use crate::jni_envelope::jstring_envelope;

/// JNI: bulk observation ingest (wifi + ble) with open enrichment session.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_ingestObservationBulkJson(
    mut env: JNIEnv,
    _class: JClass,
    wire: JByteArray,
    context_json: JString,
) -> jstring {
    let bytes = env.convert_byte_array(wire).expect("wire bytes");
    let ctx: String = env.get_string(&context_json).expect("context").into();
    let out = ingest_observation_bulk_json(&bytes, &ctx);
    env.new_string(out).expect("jni string").into_raw()
}

/// JNI: batch cell tower enrich.
#[cfg(feature = "jni")]
#[no_mangle]
pub extern "system" fn Java_net_wardriver_mobile_NativeCore_ingestCellsJson(
    mut env: JNIEnv,
    _class: JClass,
    request_json: JString,
) -> jstring {
    jstring_envelope(&mut env, &request_json, ingest_cells_json)
}
