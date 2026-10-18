//! Tier-B `diagnostics` object for `/api/stats` JSON.

use super::diagnostics_snapshot;

pub(crate) fn build_diagnostics_json() -> String {
    serde_json::to_string(&diagnostics_snapshot::build()).unwrap_or_else(|_| "{}".to_string())
}
