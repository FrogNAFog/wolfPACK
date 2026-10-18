//! `heap_caps_malloc` helpers: SPIRAM when present, internal DRAM fallback.

use esp_idf_sys as sys;
use log::warn;

use wardriver_stats as stats;

pub fn malloc_caps() -> u32 {
    if stats::free_spiram_heap_bytes() > 0 {
        sys::MALLOC_CAP_SPIRAM | sys::MALLOC_CAP_8BIT
    } else {
        warn!("spiram_heap: SPIRAM unavailable; allocation uses internal heap");
        sys::MALLOC_CAP_INTERNAL | sys::MALLOC_CAP_8BIT
    }
}

/// Allocate `bytes` from SPIRAM (internal fallback). Panics on failure.
pub fn alloc_bytes(bytes: usize) -> *mut u8 {
    if bytes == 0 {
        return core::ptr::null_mut();
    }
    let caps = malloc_caps();
    let ptr = unsafe { sys::heap_caps_malloc(bytes, caps) };
    if ptr.is_null() {
        panic!("spiram_heap: heap_caps_malloc({bytes} caps={caps:#x}) failed");
    }
    ptr.cast()
}

pub fn free_bytes(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe {
            sys::heap_caps_free(ptr.cast());
        }
    }
}

/// `Vec` with backing store allocated via [`alloc_bytes`]. Do not grow beyond `capacity`.
pub fn vec_with_capacity<T>(cap: usize) -> Vec<T> {
    if cap == 0 {
        return Vec::new();
    }
    let bytes = cap
        .checked_mul(core::mem::size_of::<T>())
        .expect("spiram_heap: capacity overflow");
    let ptr = alloc_bytes(bytes);
    unsafe { Vec::from_raw_parts(ptr.cast(), 0, cap) }
}
