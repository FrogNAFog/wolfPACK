//! Upload credentials, manual upload trigger, and alert sound settings (POST only; GET pages are static).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use embedded_io::Write as EioWrite;
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::utils::io;
use esp_idf_svc::http::client::{Configuration as HttpClientCfg, EspHttpConnection};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use wardriver_stats as stats;

use crate::config::Config;
use crate::config::DEFAULT_ALERT_BUZZER_VOLUME_PERCENT;
use crate::control::{NvsDeferOp, NvsDeferQueue, NVS_DEFER_WAIT_MS};
use crate::uploader;

use super::json_escape;
use super::pages::parse_alert_sound_mask_form;
use super::read_post_form;
use super::redirect::see_other;

/// GET `https://wdgwars.pl/api/me` with `X-API-Key` (for /uploads test).
fn wdgwars_get_me(key: &str) -> Result<String, anyhow::Error> {
    let http_cfg = HttpClientCfg {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(Duration::from_secs(15)),
        ..Default::default()
    };
    let conn = EspHttpConnection::new(&http_cfg).map_err(|e| anyhow!("{e:?}"))?;
    let mut client = HttpClient::wrap(conn);
    let headers: [(&str, &str); 3] = [
        ("accept", "application/json"),
        ("x-api-key", key),
        ("user-agent", uploader::WDG_USER_AGENT),
    ];
    let mut req = client
        .request(Method::Get, uploader::WDG_ME_URL, &headers)
        .map_err(|e| anyhow!("{e:?}"))?;
    EioWrite::flush(&mut req).map_err(|e| anyhow!("flush: {e:?}"))?;
    let mut resp = req.submit()?;
    let status = resp.status();
    let mut body = [0u8; 1200];
    let n = io::try_read_full(&mut resp, &mut body).map_err(|e| e.0)?;
    let body_s = std::str::from_utf8(&body[..n]).unwrap_or("");
    if status < 200 || status >= 300 {
        return Err(anyhow!("/api/me HTTP {status}: {body_s:?}"));
    }
    Ok(format!("{body_s} (http {status})"))
}

pub(crate) fn register_upload_routes(
    http: &mut EspHttpServer<'static>,
    nvs_defer: NvsDeferQueue,
    nvs_read: Arc<EspNvsPartition<NvsDefault>>,
) -> anyhow::Result<()> {
    let nvs_defer_uss = nvs_defer.clone();
    http.fn_handler(
        "/uploads/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 2048);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let wg_name = form.get("wg_name").cloned().unwrap_or_default();
            let wg_token = form.get("wg_token").cloned().unwrap_or_default();
            let wd_key = form.get("wd_key").cloned().unwrap_or_default();
            let wd_clear = form
                .get("wd_clear")
                .map(|v| v == "1" || v == "on")
                .unwrap_or(false);

            nvs_defer_uss
                .submit_and_wait(
                    NvsDeferOp::SaveUploadCredentials {
                        wg_name,
                        wg_token,
                        wd_key,
                        wd_clear,
                    },
                    Duration::from_millis(NVS_DEFER_WAIT_MS),
                )
                .map_err(|e| anyhow!("{e}"))?;
            see_other(req, "/uploads?msg=ok")
        },
    )?;

    let nvs_ust = Arc::clone(&nvs_read);
    http.fn_handler(
        "/uploads/test-wdgwars",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 512);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let from_form = form
                .get("wd_key")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let key = if let Some(k) = from_form {
                k
            } else {
                match Config::load(nvs_ust.as_ref()) {
                    Ok(c) => c.wdgwars_api_key.unwrap_or_default(),
                    Err(_) => String::new(),
                }
            };
            let json = if key.is_empty() {
                r#"{"ok":false,"body":"No wdgwars API key. Save one on Uploads or pass wd_key in the test POST."}"#
                    .to_string()
            } else {
                match wdgwars_get_me(&key) {
                    Ok(s) => format!(
                        r#"{{"ok":true,"body":"{}"}}"#,
                        json_escape(&s)
                    ),
                    Err(e) => format!(
                        r#"{{"ok":false,"body":"{}"}}"#,
                        json_escape(&format!("{e:#}"))
                    ),
                }
            };
            let mut resp = req.into_response(
                200,
                Some("OK"),
                &[("Content-Type", "application/json")],
            )?;
            resp.write_all(json.as_bytes())
                .map_err(|e| anyhow!("{e:?}"))
        },
    )?;

    http.fn_handler(
        "/uploads/run",
        Method::Post,
        move |req| -> Result<(), anyhow::Error> {
            stats::UPLOAD_REQUEST.store(true, Ordering::Release);
            see_other(req, "/uploads?msg=ok")
        },
    )?;

    let nvs_defer_alert = nvs_defer;
    http.fn_handler(
        "/sounds/save",
        Method::Post,
        move |mut req| -> Result<(), anyhow::Error> {
            let raw = read_post_form(&mut req, 1024);
            let form = crate::provisioning::parse_form_urlencoded_for_web(&raw);
            let mask = if form.contains_key("snd_all") {
                crate::alert_feedback::ALERT_SOUND_MASK_ALL
            } else {
                parse_alert_sound_mask_form(&form)
            };
            nvs_defer_alert
                .submit_and_wait(
                    NvsDeferOp::SaveAlertSound {
                        mask,
                        volume_percent: DEFAULT_ALERT_BUZZER_VOLUME_PERCENT,
                    },
                    Duration::from_millis(NVS_DEFER_WAIT_MS),
                )
                .map_err(|e| anyhow!("{e}"))?;
            see_other(req, "/sounds?msg=ok")
        },
    )?;

    Ok(())
}
