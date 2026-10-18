//! Host-testable MacTable capacity limits shared by control firmware and mobile enrichment.

/// Estimated heap per MAC row in the unified MacTable (`IndexMap` + `mac_table::MacEntry`).
pub const EST_BYTES_PER_MAC_TABLE_ENTRY: usize = 52;
/// Estimated heap per active geo slot (row may stay resident after release).
pub const EST_BYTES_PER_GEO_SLOT: usize = 56;

/// Concurrently geo-tracked MAC rows (revisit throttle).
pub const MAC_TABLE_GEO_SLOTS_CAP: usize = 5000;

/// Control-node combined MAC + geo heap budget when PSRAM is present.
pub const CONTROL_MAC_GEO_BUDGET_BYTES: usize = 4 * 1024 * 1024 + 1024 * 1024;

#[must_use]
pub fn mac_table_max_entries_from_budget(budget_bytes: usize) -> usize {
    (budget_bytes / EST_BYTES_PER_MAC_TABLE_ENTRY).max(256)
}

/// Split a combined byte budget into `(max_rows, geo_slots_cap)`.
#[must_use]
pub fn mac_table_limits_from_combined_budget(
    total_bytes: usize,
    geo_slots_cap: usize,
) -> (usize, usize) {
    let geo_bytes = geo_slots_cap.saturating_mul(EST_BYTES_PER_GEO_SLOT);
    let mac_bytes = total_bytes.saturating_sub(geo_bytes);
    (mac_table_max_entries_from_budget(mac_bytes), geo_slots_cap)
}

/// ESP control PSRAM MacTable tier.
#[must_use]
pub fn control_mac_table_limits() -> (usize, usize) {
    mac_table_limits_from_combined_budget(CONTROL_MAC_GEO_BUDGET_BYTES, MAC_TABLE_GEO_SLOTS_CAP)
}

/// Fixed phone control-node tier (matches [`control_mac_table_limits`]).
#[must_use]
pub fn phone_control_mac_table_limits() -> (usize, usize) {
    control_mac_table_limits()
}
