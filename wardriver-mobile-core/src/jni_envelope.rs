//! Shared JSON-envelope helper for the thin JNI wrappers.
//!
//! Standardizes the repeated "parse inbound `JString` request -> call domain
//! helper -> build outbound `JString` result envelope" pattern used by the
//! `...Json` wrappers across the `jni_*` domain modules. Each wrapper passes its
//! domain function as the closure.

#[cfg(feature = "jni")]
use jni::objects::JString;
#[cfg(feature = "jni")]
use jni::sys::jstring;
#[cfg(feature = "jni")]
use jni::JNIEnv;

/// Parse the inbound `JString` request, call the domain helper, and return the
/// outbound `JString` result envelope.
#[cfg(feature = "jni")]
#[must_use]
pub fn jstring_envelope<'local, F>(env: &mut JNIEnv<'local>, request: &JString, f: F) -> jstring
where
    F: FnOnce(&str) -> String,
{
    let input: String = env.get_string(request).expect("request json").into();
    let out = f(&input);
    env.new_string(out).expect("jni string").into_raw()
}
