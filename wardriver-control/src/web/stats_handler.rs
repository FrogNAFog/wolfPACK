//! `/api/stats` JSON handler (prebuilt snapshot; no JSON work on httpd).
//!
//! [`note_dashboard_stats_fetch`] records poll time and may flag a main-loop rebuild on
//! idle → active wardrive poll transitions (faster than the 5 s idle cadence).

use std::sync::{Arc, RwLock};

use anyhow::Result;
use embedded_svc::http::Method;
use esp_idf_svc::http::server::EspHttpServer;

use super::snapshots::{write_snapshot, WebResponseSnapshot, WebSnapshotState};

pub(crate) fn register(
    http: &mut EspHttpServer<'static>,
    snapshots: Arc<RwLock<WebSnapshotState>>,
) -> Result<(), anyhow::Error> {
    http.fn_handler(
        "/api/stats",
        Method::Get,
        move |req| -> Result<(), anyhow::Error> {
            wardriver_stats::note_dashboard_stats_fetch(wardriver_stats::now_ms());
            // Clone body under a brief read lock so httpd does not hold the lock during TCP write.
            let body = snapshots
                .read()
                .map_err(|_| anyhow::anyhow!("snapshot lock poisoned"))?
                .snapshots
                .stats
                .body
                .clone();
            let snap = WebResponseSnapshot {
                body,
                content_type: super::snapshots::CT_JSON,
            };
            write_snapshot(req, &snap)
        },
    )?;
    Ok(())
}
