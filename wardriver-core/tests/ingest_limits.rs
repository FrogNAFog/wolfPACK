//! Host tests for shared MacTable ingest capacity limits.

use wardriver_core::ingest_limits::{
    control_mac_table_limits, mac_table_limits_from_combined_budget,
    phone_control_mac_table_limits, CONTROL_MAC_GEO_BUDGET_BYTES, EST_BYTES_PER_GEO_SLOT,
    EST_BYTES_PER_MAC_TABLE_ENTRY, MAC_TABLE_GEO_SLOTS_CAP,
};

#[test]
fn phone_control_matches_control_psram_tier() {
    assert_eq!(phone_control_mac_table_limits(), control_mac_table_limits());
}

#[test]
fn control_psram_tier_geo_slots_cap() {
    let (_rows, geo) = control_mac_table_limits();
    assert_eq!(geo, MAC_TABLE_GEO_SLOTS_CAP);
    assert_eq!(geo, 5000);
}

#[test]
fn control_psram_tier_row_cap_from_budget() {
    let (rows, geo) = mac_table_limits_from_combined_budget(
        CONTROL_MAC_GEO_BUDGET_BYTES,
        MAC_TABLE_GEO_SLOTS_CAP,
    );
    assert_eq!(geo, 5000);
    let mac_bytes = CONTROL_MAC_GEO_BUDGET_BYTES - 5000 * EST_BYTES_PER_GEO_SLOT;
    assert_eq!(rows, mac_bytes / EST_BYTES_PER_MAC_TABLE_ENTRY);
    assert_eq!(rows, 95_440);
}
