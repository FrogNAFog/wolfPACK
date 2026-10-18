//! Static HTML/CSS routes (`include_str!` HTML; CSS/JSON from prebuilt snapshots).

use std::sync::{Arc, RwLock};

use embedded_svc::http::Method;
use esp_idf_svc::http::server::EspHttpServer;

use super::redirect::see_other;
use super::snapshots::{write_flash_html, write_snapshot, WebSnapshotState};
use super::{
    CAPTURE_HTML, DASHBOARD_HTML, DETECTIONS_HTML, DIAGNOSTICS_HTML, SETUP_HTML, SNIFFERS_HTML,
    SOUNDS_HTML, UPLOADS_HTML, WIGLE_FILTERS_HTML,
};

pub(crate) fn register_static_pages(
    http: &mut EspHttpServer<'static>,
    snapshots: Arc<RwLock<WebSnapshotState>>,
) -> anyhow::Result<()> {
    let css_snap = Arc::clone(&snapshots);
    http.fn_handler("/theme.css", Method::Get, move |req| {
        let snap = css_snap
            .read()
            .map_err(|_| anyhow::anyhow!("snapshot lock poisoned"))?
            .snapshots
            .theme_css
            .clone();
        write_snapshot(req, &snap)
    })?;

    let form_css_snap = Arc::clone(&snapshots);
    http.fn_handler("/simple-form.css", Method::Get, move |req| {
        let snap = form_css_snap
            .read()
            .map_err(|_| anyhow::anyhow!("snapshot lock poisoned"))?
            .snapshots
            .simple_form_css
            .clone();
        write_snapshot(req, &snap)
    })?;

    http.fn_handler("/", Method::Get, |req| {
        write_flash_html(req, DASHBOARD_HTML)
    })?;

    http.fn_handler("/sniffers", Method::Get, |req| {
        write_flash_html(req, SNIFFERS_HTML)
    })?;

    http.fn_handler("/diagnostics", Method::Get, |req| {
        write_flash_html(req, DIAGNOSTICS_HTML)
    })?;

    http.fn_handler("/detections", Method::Get, |req| {
        write_flash_html(req, DETECTIONS_HTML)
    })?;

    http.fn_handler("/capture", Method::Get, |req| {
        write_flash_html(req, CAPTURE_HTML)
    })?;

    http.fn_handler("/recon", Method::Get, |req| see_other(req, "/capture"))?;

    http.fn_handler("/setup", Method::Get, |req| {
        write_flash_html(req, SETUP_HTML)
    })?;

    http.fn_handler("/uploads", Method::Get, |req| {
        write_flash_html(req, UPLOADS_HTML)
    })?;

    http.fn_handler("/sounds", Method::Get, |req| {
        write_flash_html(req, SOUNDS_HTML)
    })?;

    http.fn_handler("/wigle-filters", Method::Get, |req| {
        write_flash_html(req, WIGLE_FILTERS_HTML)
    })?;

    // Legacy paths → consolidated pages.
    http.fn_handler("/alerts", Method::Get, |req| see_other(req, "/sounds"))?;

    http.fn_handler("/detection-ignore", Method::Get, |req| {
        see_other(req, "/detections#sec-detection-ignore")
    })?;

    // Legacy alias: same redirect as `/detection-ignore`.
    http.fn_handler("/flock-ignore", Method::Get, |req| {
        see_other(req, "/detections#sec-detection-ignore")
    })?;

    http.fn_handler("/home-zone", Method::Get, |req| {
        see_other(req, "/setup#sec-home-zone")
    })?;

    Ok(())
}
