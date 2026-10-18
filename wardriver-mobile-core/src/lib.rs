//! Host-testable mobile control brain: enrichment, geofence, coverage, JNI surface.
//!
//! Reuses [`wardriver_core`] for GPS anchor, mac_table, wigle, and scan reassembly.

pub mod cell_table;
pub mod coverage_grid;
pub mod enrichment;
pub mod enrichment_session;
pub mod geofence;
pub mod gps_anchor_mobile;
pub mod gps_timeline;
pub mod home_zone_latch;
pub mod jni_arrays;
pub mod jni_bridge;
pub mod jni_enrich;
pub mod map_geometry;
pub mod observation;
pub mod street_routing;

#[cfg(feature = "jni")]
pub mod jni;
#[cfg(feature = "jni")]
pub mod jni_csv;
#[cfg(feature = "jni")]
pub mod jni_enrich_wrap;
#[cfg(feature = "jni")]
pub mod jni_envelope;
#[cfg(feature = "jni")]
pub mod jni_geofence;
#[cfg(feature = "jni")]
pub mod jni_gps_timeline;
#[cfg(feature = "jni")]
pub mod jni_ingest;
#[cfg(feature = "jni")]
pub mod jni_packlink;

pub use wardriver_core::gps_anchor;
pub use wardriver_core::mac_table;
