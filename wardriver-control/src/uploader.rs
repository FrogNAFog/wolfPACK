//! Upload pending WiGLE-format `*.csv` from `wigle/pending/` to WiGLE and/or Watch Dogs Go Wars (wdgwars.pl).
//!
//! - WiGLE: `POST https://api.wigle.net/api/v2/file/upload` (HTTP Basic).
//! - WDGoWars: `POST https://wdgwars.pl/api/upload-csv` (header `X-API-Key`).
//!
//! Each successful destination is recorded with a pending-dir marker `*.<dest>.uploaded` next
//! to the `.csv` until every enabled destination succeeds. The CSV alone is moved to `uploaded/`;
//! markers and `*.error` snippets are removed from `pending/` (not copied to `uploaded/`).
//! Failures are also appended to `wardriver-diag.log` (`UPLOAD kind=FAIL` / `RUN_FAIL`).
//! Multipart `file` is streamed from SD to save RAM.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use embedded_io::Write as EioWrite;

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::http::Method;
use embedded_svc::utils::io;
use esp_idf_svc::http::client::{Configuration as HttpCfg, EspHttpConnection};
use log::{info, warn};

use crate::config::Config;
use crate::storage::{
    append_diag_log_line_best_effort, Paths, SdIoHandle, WIGLE_CAPTURE_MAX_FILE_BYTES,
    WIGLE_DEFAULT_FILENAME,
};
use crate::time_sync;

const WIGLE_UPLOAD_URL: &str = "https://api.wigle.net/api/v2/file/upload";
const WDG_UPLOAD_URL: &str = "https://wdgwars.pl/api/upload-csv";
pub const WDG_USER_AGENT: &str = "wolfPACK/0.1 (+wdgwars)";

/// Hard cap on file size we will attempt to upload (defensive; avoids header overflow and long uploads).
/// Capture files are rotated at [`WIGLE_CAPTURE_MAX_FILE_BYTES`]; allow small slack for FAT/OS variance.
const MAX_UPLOAD_BYTES: u64 = WIGLE_CAPTURE_MAX_FILE_BYTES + 64 * 1024;

/// SD read size per HTTPS body write (heap `Vec`; upload thread has dedicated stack).
const UPLOAD_IO_CHUNK_BYTES: usize = 128 * 1024;

/// Per-destination marker suffix, e.g. `file.csv` → `file.csv.wigle.uploaded`
pub const DEST_WIGLE: &str = "wigle";
pub const DEST_WDGWARS: &str = "wdgwars";

/// Max UTF-8 bytes stored in diag log / pending `*.error` sidecars for one failure.
const UPLOAD_ERR_TEXT_MAX: usize = 3072;

fn sanitize_upload_log_text(s: &str) -> String {
    let collapsed: String = s
        .chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' => ' ',
            _ => c,
        })
        .collect();
    if collapsed.len() <= UPLOAD_ERR_TEXT_MAX {
        return collapsed;
    }
    let mut end = UPLOAD_ERR_TEXT_MAX;
    while end > 0 && !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... [truncated]", &collapsed[..end])
}

fn log_upload_failure(dest_id: &str, csv: &Path, err: &dyn std::fmt::Display) {
    let file = csv.file_name().and_then(|n| n.to_str()).unwrap_or("-");
    let err_s = sanitize_upload_log_text(&err.to_string());
    let line = format!(
        "UPLOAD kind=FAIL dest={dest_id} file={file} utc={} t_ms={} err={err_s}",
        time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
        wardriver_stats::now_ms(),
    );
    if let Err(e) = append_diag_log_line_best_effort(&line) {
        warn!("uploader: diag log (upload fail): {e:#}");
    }
}

/// Log a whole-run upload failure (scan/read_dir, etc.) to `wardriver-diag.log`.
pub fn log_upload_run_failure(context: &str, err: &dyn std::fmt::Display) {
    let err_s = sanitize_upload_log_text(&err.to_string());
    let line = format!(
        "UPLOAD kind=RUN_FAIL context={context} utc={} t_ms={} err={err_s}",
        time_sync::now_utc_string().unwrap_or_else(|| "-".into()),
        wardriver_stats::now_ms(),
    );
    if let Err(e) = append_diag_log_line_best_effort(&line) {
        warn!("uploader: diag log (upload run fail): {e:#}");
    }
}

fn eio_write_all<W: EioWrite>(w: &mut W, mut data: &[u8]) -> Result<()> {
    while !data.is_empty() {
        let n = w.write(data).map_err(|e| anyhow!("write: {e:?}"))?;
        if n == 0 {
            anyhow::bail!("short write");
        }
        data = &data[n..];
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct UploadSummary {
    pub wigle_ok: u32,
    pub wigle_failed: u32,
    pub wdgwars_ok: u32,
    pub wdgwars_failed: u32,
}

impl UploadSummary {
    /// Any destination had at least one failed upload attempt this run.
    pub fn had_failures(&self) -> bool {
        self.wigle_failed > 0 || self.wdgwars_failed > 0
    }

    /// SSD1306 title for the completion screen.
    pub fn oled_done_title(&self) -> &'static str {
        if self.had_failures() {
            "UPLOAD ERROR"
        } else {
            "UPLOAD DONE"
        }
    }

    /// Short OLED / alert subtitle, e.g. `W3 F1 D2 F0`.
    pub fn oled_subtitle(&self) -> String {
        format!(
            "W{} F{} D{} F{}",
            self.wigle_ok, self.wigle_failed, self.wdgwars_ok, self.wdgwars_failed
        )
    }
}

pub use wardriver_stats::upload_progress::{UploadDest, UploadProgress, UploadProgressKind};

fn emit_upload_progress<F>(
    on_progress: &mut Option<F>,
    kind: UploadProgressKind,
    summary: &UploadSummary,
) where
    F: FnMut(UploadProgress),
{
    let p = UploadProgress {
        kind,
        wigle_ok: summary.wigle_ok,
        wigle_fail: summary.wigle_failed,
        wdgwars_ok: summary.wdgwars_ok,
        wdgwars_fail: summary.wdgwars_failed,
    };
    UploadProgress::emit(on_progress, p);
    wardriver_stats::set_upload_progress(&p);
}

/// Title + subtitle for SSD1306 when an upload run completes.
pub fn upload_oled_done_message(summary: &UploadSummary) -> (String, String) {
    (
        summary.oled_done_title().to_string(),
        summary.oled_subtitle(),
    )
}

/// Title + subtitle for SSD1306 during an upload run.
pub fn upload_oled_message(p: &UploadProgress) -> (String, String) {
    match p.kind {
        UploadProgressKind::Connecting => ("UPLOAD".into(), "connecting...".into()),
        UploadProgressKind::Scanning => ("UPLOAD".into(), "scanning...".into()),
        UploadProgressKind::Uploading {
            file_index,
            file_total,
            dest,
            byte_pct,
        } => {
            // 255 means between files / before body transfer; 0-100 shows HTTP progress.
            let title = if byte_pct <= 100 {
                format!("UPLOAD {byte_pct}%")
            } else {
                "UPLOAD".into()
            };
            let subtitle = format!("{file_index}/{file_total} {}", dest.oled_label());
            (title, subtitle)
        }
        UploadProgressKind::NothingPending => ("UPLOAD".into(), "0 pending".into()),
        UploadProgressKind::Done => {
            let summary = UploadSummary {
                wigle_ok: p.wigle_ok,
                wigle_failed: p.wigle_fail,
                wdgwars_ok: p.wdgwars_ok,
                wdgwars_failed: p.wdgwars_fail,
            };
            upload_oled_done_message(&summary)
        }
        UploadProgressKind::Failed => ("UPLOAD ERROR".into(), "failed".into()),
    }
}

fn marker_basename(csv: &Path, dest_id: &str) -> String {
    let base = csv
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(WIGLE_DEFAULT_FILENAME);
    format!("{base}.{dest_id}.uploaded")
}

fn marker_path(csv: &Path, dest_id: &str) -> PathBuf {
    let name = marker_basename(csv, dest_id);
    csv.parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn error_path(csv: &Path, dest_id: &str) -> PathBuf {
    let base = csv
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(WIGLE_DEFAULT_FILENAME);
    let name = format!("{base}.{dest_id}.error");
    csv.parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn touch_marker(sd_io: &SdIoHandle, p: &Path) -> Result<()> {
    sd_io
        .rpc_write_empty_file(p.to_path_buf())
        .with_context(|| format!("create marker {p:?}"))
}

fn write_error_snippet(sd_io: &SdIoHandle, csv: &Path, dest_id: &str, err: &dyn std::fmt::Display) {
    let p = error_path(csv, dest_id);
    let text = sanitize_upload_log_text(&err.to_string());
    let _ = sd_io.rpc_write_atomic_text(p, text.into_bytes());
}

/// True when `paths.pending` contains at least one WiGLE capture file.
pub fn pending_has_uploadable_file(sd_io: &SdIoHandle, _paths: &Paths) -> Result<bool> {
    match sd_io.rpc_list_pending_csv() {
        Ok(files) => Ok(!files.is_empty()),
        Err(e) => Err(anyhow!("list pending: {e}")),
    }
}

struct SdSessionReader<'a> {
    sd_io: &'a SdIoHandle,
    size: u64,
    offset: u64,
}

impl<'a> SdSessionReader<'a> {
    fn new(sd_io: &'a SdIoHandle, size: u64) -> Self {
        Self {
            sd_io,
            size,
            offset: 0,
        }
    }
}

impl Read for SdSessionReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.offset >= self.size {
            return Ok(0);
        }
        let want = buf
            .len()
            .min(self.size.saturating_sub(self.offset) as usize);
        let chunk = self.sd_io.rpc_upload_read_chunk(want)?;
        let n = chunk.len();
        if n == 0 {
            return Ok(0);
        }
        buf[..n].copy_from_slice(&chunk);
        self.offset = self.offset.saturating_add(n as u64);
        Ok(n)
    }
}

/// Closes the SD-worker upload read session on drop.
struct UploadReadGuard<'a> {
    sd_io: &'a SdIoHandle,
}

impl<'a> UploadReadGuard<'a> {
    fn open(sd_io: &'a SdIoHandle, path: PathBuf) -> std::io::Result<Self> {
        sd_io.rpc_upload_read_open(path)?;
        Ok(Self { sd_io })
    }
}

impl Drop for UploadReadGuard<'_> {
    fn drop(&mut self) {
        self.sd_io.rpc_upload_read_close();
    }
}

enum ActiveDest {
    Wigle { name: String, token: String },
    Wdgwars { key: String },
}

impl ActiveDest {
    fn id(&self) -> &'static str {
        match self {
            Self::Wigle { .. } => DEST_WIGLE,
            Self::Wdgwars { .. } => DEST_WDGWARS,
        }
    }

    fn upload(
        &self,
        client: &mut HttpClient<EspHttpConnection>,
        file_name: &str,
        size: u64,
        path: &Path,
        body: &mut dyn Read,
        on_bytes: Option<&mut dyn FnMut(u64, u64)>,
    ) -> Result<()> {
        match self {
            Self::Wigle { name, token } => {
                upload_wigle(client, name, token, file_name, size, path, body, on_bytes)
            }
            Self::Wdgwars { key } => {
                upload_wdgwars(client, key, file_name, size, path, body, on_bytes)
            }
        }
    }
}

struct DestHttpClients {
    wigle: Option<HttpClient<EspHttpConnection>>,
    wdgwars: Option<HttpClient<EspHttpConnection>>,
}

impl DestHttpClients {
    fn new(dests: &[ActiveDest], http_cfg: &HttpCfg) -> Result<Self> {
        let mut out = Self {
            wigle: None,
            wdgwars: None,
        };
        for dest in dests {
            out.ensure(dest, http_cfg)?;
        }
        Ok(out)
    }

    fn ensure(&mut self, dest: &ActiveDest, http_cfg: &HttpCfg) -> Result<()> {
        match dest {
            ActiveDest::Wigle { .. } if self.wigle.is_none() => {
                self.wigle = Some(HttpClient::wrap(EspHttpConnection::new(http_cfg)?));
            }
            ActiveDest::Wdgwars { .. } if self.wdgwars.is_none() => {
                self.wdgwars = Some(HttpClient::wrap(EspHttpConnection::new(http_cfg)?));
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, dest: &ActiveDest, http_cfg: &HttpCfg) -> Result<()> {
        match dest {
            ActiveDest::Wigle { .. } => {
                self.wigle = Some(HttpClient::wrap(EspHttpConnection::new(http_cfg)?));
            }
            ActiveDest::Wdgwars { .. } => {
                self.wdgwars = Some(HttpClient::wrap(EspHttpConnection::new(http_cfg)?));
            }
        }
        Ok(())
    }

    fn client_mut(&mut self, dest: &ActiveDest) -> Option<&mut HttpClient<EspHttpConnection>> {
        match dest {
            ActiveDest::Wigle { .. } => self.wigle.as_mut(),
            ActiveDest::Wdgwars { .. } => self.wdgwars.as_mut(),
        }
    }
}

fn marker_exists(markers: &HashSet<String>, csv: &Path, dest_id: &str) -> bool {
    markers.contains(&marker_basename(csv, dest_id))
}

fn all_markers_for_csv_exist(markers: &HashSet<String>, csv: &Path, dest_ids: &[&str]) -> bool {
    dest_ids.iter().all(|d| marker_exists(markers, csv, d))
}

fn pending_marker_names(sd_io: &SdIoHandle, pending: &Path) -> HashSet<String> {
    sd_io
        .rpc_read_dir(pending.to_path_buf())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn upload_pending_with_progress<F>(
    cfg: &Config,
    paths: &Paths,
    sd_io: Arc<SdIoHandle>,
    on_progress: Option<F>,
) -> Result<UploadSummary>
where
    F: FnMut(UploadProgress),
{
    let mut on_progress = on_progress;
    let sd_io_ref = sd_io.as_ref();

    let mut dests: Vec<ActiveDest> = Vec::new();
    if cfg.wigle_configured() {
        dests.push(ActiveDest::Wigle {
            name: cfg
                .wigle_api_name
                .as_deref()
                .ok_or_else(|| anyhow!("internal: wigle_configured but no name"))?
                .to_string(),
            token: cfg
                .wigle_api_token
                .as_deref()
                .ok_or_else(|| anyhow!("internal: wigle_configured but no token"))?
                .to_string(),
        });
    }
    if cfg.wdgwars_configured() {
        dests.push(ActiveDest::Wdgwars {
            key: cfg
                .wdgwars_api_key
                .as_deref()
                .ok_or_else(|| anyhow!("internal: wdgwars_configured but no key"))?
                .to_string(),
        });
    }

    if dests.is_empty() {
        info!("uploader: no upload destinations configured; skipping");
        return Ok(UploadSummary::default());
    }

    info!(
        "uploader: start chunk_bytes={UPLOAD_IO_CHUNK_BYTES} wall_clock_trusted={} free_internal_heap={} free_spiram_heap={} min_ever_free_heap={}",
        time_sync::wall_clock_trusted(),
        wardriver_stats::free_internal_heap_bytes(),
        wardriver_stats::free_spiram_heap_bytes(),
        wardriver_stats::min_ever_free_heap_bytes(),
    );

    let dest_ids: Vec<&str> = dests.iter().map(|d| d.id()).collect();
    let mut summary = UploadSummary::default();

    let http_cfg = HttpCfg {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        timeout: Some(std::time::Duration::from_secs(120)),
        ..Default::default()
    };

    let pending = &paths.pending;
    if !wardriver_stats::SD_MOUNT_OK.load(Ordering::Acquire) {
        info!("uploader: SD not mounted; skipping: {:?}", pending);
        emit_upload_progress(
            &mut on_progress,
            UploadProgressKind::NothingPending,
            &summary,
        );
        return Ok(UploadSummary::default());
    }

    emit_upload_progress(&mut on_progress, UploadProgressKind::Scanning, &summary);

    let files = sd_io_ref
        .rpc_list_pending_csv()
        .with_context(|| format!("list pending {:?}", pending))?;
    let file_total = files.len() as u32;
    wardriver_stats::UPLOAD_FILES_TOTAL.store(file_total, Ordering::Release);

    if file_total == 0 {
        emit_upload_progress(
            &mut on_progress,
            UploadProgressKind::NothingPending,
            &summary,
        );
        wardriver_stats::record_upload(0, 0, 0, 0);
        return Ok(UploadSummary::default());
    }

    let mut pending_markers = pending_marker_names(sd_io_ref, pending);

    // One TLS client per destination for the whole run (reset that client on HTTP failure).
    let upload_result: Result<()> = {
        let mut clients = DestHttpClients::new(&dests, &http_cfg)?;
        for (file_idx, path) in files.iter().enumerate() {
            let file_index = (file_idx as u32).saturating_add(1);

            if all_markers_for_csv_exist(&pending_markers, path, &dest_ids) {
                if let Err(e) =
                    move_to_uploaded(sd_io.as_ref(), paths, path, &dest_ids, &pending_markers)
                {
                    warn!("uploader: move to uploaded failed (leaving pending for retry): {e:#}");
                }
                continue;
            }

            let needs_upload = dests
                .iter()
                .any(|d| !marker_exists(&pending_markers, path, d.id()));
            if !needs_upload {
                continue;
            }

            let size = sd_io_ref
                .rpc_file_len(path.to_path_buf())
                .with_context(|| format!("metadata {:?}", path))?;
            if size > MAX_UPLOAD_BYTES {
                warn!(
                    "uploader: skip {:?}: {} bytes (> {} cap)",
                    path, size, MAX_UPLOAD_BYTES
                );
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(WIGLE_DEFAULT_FILENAME);

            let file_t0 = wardriver_stats::now_ms();
            let _read_guard = UploadReadGuard::open(sd_io_ref, path.to_path_buf())
                .with_context(|| format!("upload read open {:?}", path))?;

            for dest in &dests {
                let id = dest.id();
                if marker_exists(&pending_markers, path, id) {
                    continue;
                }
                sd_io_ref
                    .rpc_upload_read_seek(0)
                    .with_context(|| format!("upload read seek 0 {:?}", path))?;

                let upload_dest = UploadDest::from_id(id).unwrap_or(UploadDest::Wigle);
                emit_upload_progress(
                    &mut on_progress,
                    UploadProgressKind::Uploading {
                        file_index,
                        file_total,
                        dest: upload_dest,
                        byte_pct: 0,
                    },
                    &summary,
                );

                let client = clients
                    .client_mut(dest)
                    .ok_or_else(|| anyhow!("internal: no HTTP client for dest {id}"))?;
                let mut session_reader = SdSessionReader::new(sd_io_ref, size);
                let mut on_bytes = |sent: u64, total: u64| {
                    let byte_pct = if total > 0 {
                        ((sent * 100 / total).min(100)) as u8
                    } else {
                        0
                    };
                    emit_upload_progress(
                        &mut on_progress,
                        UploadProgressKind::Uploading {
                            file_index,
                            file_total,
                            dest: upload_dest,
                            byte_pct,
                        },
                        &summary,
                    );
                };
                let r = dest.upload(
                    client,
                    file_name,
                    size,
                    path,
                    &mut session_reader,
                    Some(&mut on_bytes),
                );
                match r {
                    Ok(()) => {
                        touch_marker(sd_io.as_ref(), &marker_path(path, id))?;
                        pending_markers.insert(marker_basename(path, id));
                        match dest {
                            ActiveDest::Wigle { .. } => {
                                summary.wigle_ok = summary.wigle_ok.saturating_add(1);
                            }
                            ActiveDest::Wdgwars { .. } => {
                                summary.wdgwars_ok = summary.wdgwars_ok.saturating_add(1);
                            }
                        }
                    }
                    Err(e) => {
                        match dest {
                            ActiveDest::Wigle { .. } => {
                                summary.wigle_failed = summary.wigle_failed.saturating_add(1);
                            }
                            ActiveDest::Wdgwars { .. } => {
                                summary.wdgwars_failed = summary.wdgwars_failed.saturating_add(1);
                            }
                        }
                        log_upload_failure(id, path, &e);
                        write_error_snippet(sd_io.as_ref(), path, id, &e);
                        warn!("upload failed for {:?} dest={id}: {e:#}", path);
                        clients.reset(dest, &http_cfg)?;
                    }
                }
                emit_upload_progress(
                    &mut on_progress,
                    UploadProgressKind::Uploading {
                        file_index,
                        file_total,
                        dest: upload_dest,
                        byte_pct: 255,
                    },
                    &summary,
                );
            }

            let wall_ms = wardriver_stats::now_ms().saturating_sub(file_t0);
            info!("uploader: {:?} size={} wall_ms={}", path, size, wall_ms);

            if all_markers_for_csv_exist(&pending_markers, path, &dest_ids) {
                if let Err(e) =
                    move_to_uploaded(sd_io.as_ref(), paths, path, &dest_ids, &pending_markers)
                {
                    warn!("uploader: move to uploaded failed (leaving pending for retry): {e:#}");
                }
            }
        }
        Ok(())
    };

    match &upload_result {
        Ok(()) => emit_upload_progress(&mut on_progress, UploadProgressKind::Done, &summary),
        Err(_) => {
            emit_upload_progress(&mut on_progress, UploadProgressKind::Failed, &summary);
            wardriver_stats::set_upload_phase(wardriver_stats::UPLOAD_PHASE_FAILED);
        }
    }

    wardriver_stats::record_upload(
        summary.wigle_ok,
        summary.wigle_failed,
        summary.wdgwars_ok,
        summary.wdgwars_failed,
    );
    upload_result?;
    Ok(summary)
}

/// Shared upload core: size cap, multipart framing, chunked streaming, flush,
/// submit, and response-body read. Headers and response handling are supplied
/// per-destination so each dest's exact header set (auth style, user-agent
/// presence, order) and accept logic stay byte-identical.
fn http_post_lines(
    client: &mut HttpClient<EspHttpConnection>,
    url: &str,
    send_label: &str,
    status_label: &str,
    file_name: &str,
    size: u64,
    path: &Path,
    body: &mut dyn Read,
    mut on_bytes: Option<&mut dyn FnMut(u64, u64)>,
    build_headers: &mut dyn FnMut(&str, u64) -> Vec<(String, String)>,
    on_response: &mut dyn FnMut(u16, &str) -> Result<()>,
) -> Result<()> {
    if size > MAX_UPLOAD_BYTES {
        anyhow::bail!(
            "refusing upload: file is {} bytes (> {} bytes cap)",
            size,
            MAX_UPLOAD_BYTES
        );
    }

    let boundary = format!(
        "wardriver-{:x}",
        (esp_idf_svc::systime::EspSystemTime {}).now().as_millis()
    );

    let pre = format!(
        "--{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{fn}\"\r\nContent-Type: text/csv\r\n\r\n",
        b = boundary,
        fn = file_name
    );
    let post = format!("\r\n--{b}--\r\n", b = boundary);
    let content_len: u64 = (pre.len() as u64)
        .saturating_add(size)
        .saturating_add(post.len() as u64);

    let headers_owned = build_headers(&boundary, content_len);
    let headers: Vec<(&str, &str)> = headers_owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let mut req = client.request(Method::Post, url, &headers)?;
    eio_write_all(&mut req, pre.as_bytes())?;

    let mut buf = vec![0u8; UPLOAD_IO_CHUNK_BYTES];
    let mut sent: u64 = 0;
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        eio_write_all(&mut req, &buf[..n])?;
        sent = sent.saturating_add(n as u64);
        if let Some(cb) = on_bytes.as_mut() {
            cb(sent, size);
        }
    }

    eio_write_all(&mut req, post.as_bytes())?;
    EioWrite::flush(&mut req).map_err(|e| anyhow!("flush: {e:?}"))?;

    info!("-> {send_label} upload {:?}", path);
    let mut resp = req.submit()?;
    let status = resp.status();
    info!("<- {status_label} status {status}");

    let mut resp_body = vec![0u8; 2048];
    let n = io::try_read_full(&mut resp, &mut resp_body).map_err(|e| e.0)?;
    let body_s = std::str::from_utf8(&resp_body[..n]).unwrap_or("");

    on_response(status, body_s)
}

fn upload_wigle(
    client: &mut HttpClient<EspHttpConnection>,
    api_name: &str,
    api_token: &str,
    file_name: &str,
    size: u64,
    path: &Path,
    body: &mut dyn Read,
    on_bytes: Option<&mut dyn FnMut(u64, u64)>,
) -> Result<()> {
    let name = api_name.to_string();
    let token = api_token.to_string();
    let mut build_headers = |boundary: &str, content_len: u64| -> Vec<(String, String)> {
        let basic = format!("{name}:{token}");
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(basic.as_bytes())
        );
        let len_hdr = content_len.to_string();
        let ctype = format!("multipart/form-data; boundary={boundary}");
        vec![
            ("accept".to_string(), "application/json".to_string()),
            ("authorization".to_string(), auth),
            ("content-type".to_string(), ctype),
            ("content-length".to_string(), len_hdr),
        ]
    };
    let mut on_response = |status: u16, body_s: &str| -> Result<()> {
        if status < 200 || status >= 300 {
            return Err(anyhow!(
                "wigle HTTP {status}: {}",
                sanitize_upload_log_text(body_s)
            ));
        }
        if !wardriver_core::wdgwars_upload::wigle_upload_accepted(status, body_s) {
            return Err(anyhow!(
                "wigle upload response did not indicate success: {}",
                sanitize_upload_log_text(body_s)
            ));
        }
        Ok(())
    };
    http_post_lines(
        client,
        WIGLE_UPLOAD_URL,
        "WiGLE",
        "WiGLE",
        file_name,
        size,
        path,
        body,
        on_bytes,
        &mut build_headers,
        &mut on_response,
    )
}

fn upload_wdgwars(
    client: &mut HttpClient<EspHttpConnection>,
    api_key: &str,
    file_name: &str,
    size: u64,
    path: &Path,
    body: &mut dyn Read,
    on_bytes: Option<&mut dyn FnMut(u64, u64)>,
) -> Result<()> {
    let key = api_key.to_string();
    let mut build_headers = |boundary: &str, content_len: u64| -> Vec<(String, String)> {
        let ctype = format!("multipart/form-data; boundary={boundary}");
        let len_str = content_len.to_string();
        vec![
            ("accept".to_string(), "application/json".to_string()),
            ("x-api-key".to_string(), key.clone()),
            ("user-agent".to_string(), WDG_USER_AGENT.to_string()),
            ("content-type".to_string(), ctype),
            ("content-length".to_string(), len_str),
        ]
    };
    let mut on_response = |status: u16, body_s: &str| -> Result<()> {
        if wardriver_core::wdgwars_upload::wdgwars_upload_accepted(status, body_s) {
            if status < 200 || status >= 300 {
                let preview = wardriver_core::wdgwars_upload::wdgwars_body_preview(body_s);
                if status == 409 {
                    info!("wdgwars: HTTP 409 duplicate treated as uploaded (preview: {preview:?})");
                } else {
                    info!("wdgwars: HTTP {status} accepted with success signal (preview: {preview:?})");
                }
            }
            return Ok(());
        }
        Err(anyhow!(
            "wdgwars HTTP {status}: {}",
            sanitize_upload_log_text(body_s)
        ))
    };
    http_post_lines(
        client,
        WDG_UPLOAD_URL,
        "wdgwars.pl",
        "wdgwars",
        file_name,
        size,
        path,
        body,
        on_bytes,
        &mut build_headers,
        &mut on_response,
    )
}

fn move_to_uploaded(
    sd_io: &SdIoHandle,
    paths: &Paths,
    csv: &Path,
    dest_ids: &[&str],
    markers: &HashSet<String>,
) -> Result<()> {
    let file_name = csv
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("bad path"))?;
    let parent = csv
        .parent()
        .ok_or_else(|| anyhow!("csv has no parent dir"))?;

    for id in dest_ids {
        let m = marker_path(csv, id);
        if marker_exists(markers, csv, id) {
            if let Err(e) = sd_io.rpc_remove(m) {
                warn!("uploader: remove pending marker: {e:#}");
            }
        }
    }

    let prefix = format!("{file_name}.");
    if let Ok(entries) = sd_io.rpc_read_dir(parent.to_path_buf()) {
        for p in entries {
            if p.as_path() == csv {
                continue;
            }
            if let Some(n) = p.file_name().and_then(|x| x.to_str()) {
                if n.starts_with(&prefix) && n.ends_with(".error") {
                    if let Err(e) = sd_io.rpc_remove(p) {
                        warn!("uploader: remove error file: {e:#}");
                    }
                }
            }
        }
    }

    let dest_main = paths.uploaded.join(file_name);
    sd_io
        .rpc_rename(csv.to_path_buf(), dest_main.clone())
        .with_context(|| format!("rename {:?} -> {:?}", csv, dest_main))?;
    Ok(())
}

/// Public URL and header setup for wdgwars `GET /api/me` (used from `web.rs`).
pub const WDG_ME_URL: &str = "https://wdgwars.pl/api/me";
