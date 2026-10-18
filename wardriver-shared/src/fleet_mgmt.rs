//! Compile-time fleet management SoftAP credentials (from build.rs / `.fleet/mgmt_ap.toml`).
include!(concat!(env!("OUT_DIR"), "/mgmt_ap.rs"));
