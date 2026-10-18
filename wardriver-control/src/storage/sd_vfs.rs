//! Worker-only FAT/VFS helpers for the mounted SD card.
//!
//! **All** `std::fs` access on [`super::MOUNT_POINT`] must live in this module.

use std::collections::HashSet;
use std::fs::{create_dir_all, read_dir, remove_file, rename, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Result as IoResult, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use log::{info, warn};
use wardriver_shared::task_wdt;

use super::{
    format_mac_colon, is_wigle_capture_file, validate_ssid_denylist_pack, CleanupResult, Paths,
    DIAG_LOG_KEEP_FILES, DIAG_LOG_MAX_BYTES, DIAG_LOG_RELPATH, MOUNT_POINT,
};
use crate::csv;
use wardriver_stats as stats;

/// True when the control boot path marked SD as mounted.
pub(crate) fn sd_mounted() -> bool {
    stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire)
}

fn sd_not_mounted_err() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, "sd not mounted")
}

pub(crate) fn read_text(path: &Path) -> IoResult<Option<String>> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Atomic text write: `*.tmp` + flush + sync + rename.
pub(crate) fn write_atomic_text(path: &Path, body: &[u8]) -> IoResult<()> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!("{fname}.tmp"));
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(body)?;
        f.flush()?;
        f.sync_data()?;
    }
    let _ = remove_file(path);
    rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn create_dirs(paths: &[PathBuf]) -> IoResult<()> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    for p in paths {
        create_dir_all(p)?;
    }
    Ok(())
}

pub(crate) fn ensure_standard_layout() -> IoResult<()> {
    let root = super::sd_data_root();
    create_dirs(&[
        root.join("config"),
        root.join("exclusions/wigle"),
        root.join("exclusions/flock"),
        Paths::new().pending.clone(),
        Paths::new().uploaded.clone(),
        super::DetectionPaths::new().pending,
        super::MdetectionPaths::new().pending,
        super::recon_wifiprobes_dir(),
    ])
}

fn diag_log_path_for_index(base: &Path, idx: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", base.display(), idx))
}

fn rotate_diag_logs(base: &Path) -> IoResult<()> {
    if DIAG_LOG_KEEP_FILES < 2 {
        return Ok(());
    }
    let oldest = diag_log_path_for_index(base, DIAG_LOG_KEEP_FILES - 1);
    let _ = remove_file(&oldest);
    for i in (1..=(DIAG_LOG_KEEP_FILES - 2)).rev() {
        let from = diag_log_path_for_index(base, i);
        let to = diag_log_path_for_index(base, i + 1);
        let _ = rename(&from, &to);
    }
    let first = diag_log_path_for_index(base, 1);
    let _ = rename(base, &first);
    Ok(())
}

pub(crate) fn write_diag_log_line(line: &str) -> IoResult<()> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    let paths = Paths::new();
    create_dir_all(&paths.root)?;
    let p = paths.root.join(DIAG_LOG_RELPATH);
    if let Ok(m) = std::fs::metadata(&p) {
        if m.len() > DIAG_LOG_MAX_BYTES {
            rotate_diag_logs(&p)?;
            let mut f = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&p)?;
            writeln!(
                f,
                "---- log rotated to .1, was {} bytes, uptime {} ms ----",
                m.len(),
                stats::now_ms()
            )?;
            f.flush()?;
            f.sync_data()?;
        }
    }
    let mut f = OpenOptions::new().create(true).append(true).open(&p)?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    f.flush()?;
    f.sync_data()?;
    stats::diag_log_line_ok();
    Ok(())
}

const DIAG_SYNC_EVERY_LINES: u32 = 5;
const DIAG_SYNC_EVERY_MS: u64 = 2_000;

/// Persistent diag log handle for the sd-io worker (batched `sync_data`).
pub(crate) struct DiagLogWriter {
    path: PathBuf,
    file: File,
    file_len: u64,
    lines_since_sync: u32,
    last_sync_ms: u64,
}

impl DiagLogWriter {
    pub(crate) fn open() -> IoResult<Self> {
        if !sd_mounted() {
            return Err(sd_not_mounted_err());
        }
        let paths = Paths::new();
        create_dir_all(&paths.root)?;
        let path = paths.root.join(DIAG_LOG_RELPATH);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            path,
            file,
            file_len,
            lines_since_sync: 0,
            last_sync_ms: stats::now_ms(),
        })
    }

    pub(crate) fn append_line(&mut self, line: &str) -> IoResult<()> {
        self.maybe_rotate()?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file_len = self.file_len.saturating_add(line.len() as u64 + 1);
        self.lines_since_sync = self.lines_since_sync.saturating_add(1);
        self.file.flush()?;
        let now = stats::now_ms();
        let elapsed = if self.last_sync_ms == 0 {
            u64::MAX
        } else {
            now.saturating_sub(self.last_sync_ms)
        };
        if self.lines_since_sync >= DIAG_SYNC_EVERY_LINES || elapsed >= DIAG_SYNC_EVERY_MS {
            task_wdt::reset_if_subscribed();
            self.file.sync_data()?;
            self.lines_since_sync = 0;
            self.last_sync_ms = now;
        }
        stats::diag_log_line_ok();
        Ok(())
    }

    fn maybe_rotate(&mut self) -> IoResult<()> {
        if self.file_len <= DIAG_LOG_MAX_BYTES {
            return Ok(());
        }
        let old_len = self.file_len;
        self.file.flush()?;
        self.file.sync_data()?;
        rotate_diag_logs(&self.path)?;
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        writeln!(
            f,
            "---- log rotated to .1, was {} bytes, uptime {} ms ----",
            old_len,
            stats::now_ms()
        )?;
        f.flush()?;
        f.sync_data()?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.file_len = self.file.metadata().map(|m| m.len()).unwrap_or(0);
        self.lines_since_sync = 0;
        self.last_sync_ms = stats::now_ms();
        Ok(())
    }
}

pub(crate) fn count_pending_wigle_csvs() -> u32 {
    if !sd_mounted() {
        return 0;
    }
    let pending = Paths::new().pending;
    let Ok(rd) = read_dir(&pending) else {
        return 0;
    };
    let mut n = 0u32;
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_file() && is_wigle_capture_file(&path) {
            n = n.saturating_add(1);
        }
    }
    n
}

fn is_header_only_wiglecsv(path: &Path) -> IoResult<bool> {
    let f = File::open(path)?;
    let mut non_empty: Vec<String> = Vec::with_capacity(3);
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        non_empty.push(line);
        if non_empty.len() > 2 {
            return Ok(false);
        }
    }
    if non_empty.len() != 2 {
        return Ok(false);
    }
    let first = non_empty[0].trim_start_matches('\u{feff}').trim();
    let second = non_empty[1].trim();
    let ok = first.starts_with(csv::WIGLE_VERSION) && second == csv::CSV_HEADER;
    Ok(ok)
}

/// Invokes `on_yield` after each examined file so the SD worker can service RPC while cleanup runs.
pub(crate) fn cleanup_header_only_wiglecsv_files_yield(
    mut on_yield: impl FnMut(),
) -> CleanupResult {
    let paths = Paths::new();
    let mut out = CleanupResult::default();
    if !sd_mounted() {
        return out;
    }
    for dir in [&paths.pending, &paths.uploaded] {
        let Ok(rd) = read_dir(dir) else {
            continue;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if !p.is_file() || !is_wigle_capture_file(&p) {
                continue;
            }
            out.examined = out.examined.saturating_add(1);
            if out.examined & 0x7 == 0 {
                task_wdt::reset_if_subscribed();
            }
            on_yield();
            match is_header_only_wiglecsv(&p) {
                Ok(true) => match remove_file(&p) {
                    Ok(()) => out.deleted = out.deleted.saturating_add(1),
                    Err(_) => out.errors = out.errors.saturating_add(1),
                },
                Ok(false) => {}
                Err(_) => out.errors = out.errors.saturating_add(1),
            }
        }
    }
    out
}

pub(crate) fn file_len(path: &Path) -> IoResult<u64> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    Ok(std::fs::metadata(path)?.len())
}

/// Open upload-read handle for sequential HTTPS streaming (worker thread only).
pub(crate) struct UploadReadFile {
    file: File,
}

impl UploadReadFile {
    pub(crate) fn open(path: &Path) -> IoResult<Self> {
        if !sd_mounted() {
            return Err(sd_not_mounted_err());
        }
        Ok(Self {
            file: File::open(path)?,
        })
    }

    pub(crate) fn seek(&mut self, offset: u64) -> IoResult<()> {
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }

    pub(crate) fn read_chunk(&mut self, len: usize) -> IoResult<Vec<u8>> {
        let mut out = vec![0u8; len];
        let n = self.file.read(&mut out)?;
        out.truncate(n);
        Ok(out)
    }
}

pub(crate) fn rename_path(from: &Path, to: &Path) -> IoResult<()> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    if let Some(parent) = to.parent() {
        create_dir_all(parent)?;
    }
    rename(from, to)
}

pub(crate) fn remove_path(path: &Path) -> IoResult<()> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    remove_file(path)
}

pub(crate) fn write_empty_file(path: &Path) -> IoResult<()> {
    write_atomic_text(path, &[])
}

pub(crate) fn create_new_file(path: &Path) -> IoResult<File> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .or_else(|_| {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(path)
        })
}

pub(crate) fn read_dir_paths(dir: &Path) -> IoResult<Vec<PathBuf>> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    let mut out = Vec::new();
    for ent in read_dir(dir)?.flatten() {
        out.push(ent.path());
    }
    Ok(out)
}

pub(crate) struct ReconWriter {
    pub file: File,
    pub meta_path: PathBuf,
}

pub(crate) fn recon_open(
    pcap_path: &Path,
    meta_path: &Path,
    initial: &[u8],
) -> IoResult<ReconWriter> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    if let Some(parent) = pcap_path.parent() {
        create_dir_all(parent)?;
    }
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(pcap_path)?;
    f.write_all(initial)?;
    f.flush()?;
    f.sync_data()?;
    Ok(ReconWriter {
        file: f,
        meta_path: meta_path.to_path_buf(),
    })
}

pub(crate) fn recon_append(writer: &mut ReconWriter, data: &[u8]) -> IoResult<()> {
    writer.file.write_all(data)?;
    writer.file.flush()?;
    Ok(())
}

pub(crate) fn recon_close(mut writer: ReconWriter, meta_json: &str) -> IoResult<()> {
    writer.file.flush()?;
    writer.file.sync_data()?;
    drop(writer.file);
    write_atomic_text(&writer.meta_path, meta_json.as_bytes())
}

pub(crate) fn pending_csv_abs_names() -> IoResult<Vec<PathBuf>> {
    if !sd_mounted() {
        return Err(sd_not_mounted_err());
    }
    let pending = Paths::new().pending;
    let mut files = Vec::new();
    for ent in read_dir(&pending)?.flatten() {
        let path = ent.path();
        if path.is_file() && is_wigle_capture_file(&path) {
            files.push(path);
        }
    }
    Ok(files)
}
