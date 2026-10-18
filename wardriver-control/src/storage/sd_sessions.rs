//! WiGLE / detection / mdetection session writers (SD worker thread only).

use std::fs::File;
use std::io::{Result as IoResult, Write};
use std::path::PathBuf;

use log::{info, warn};

use super::sd_vfs;
use super::{
    DetectionPaths, MdetectionPaths, Paths, RemoteIdPaths, WIGLE_CAPTURE_EXT,
    WIGLE_CAPTURE_MAX_FILE_BYTES,
};
use crate::csv;
use wardriver_stats as stats;

/// Rows written since last successful `sync_data()` before flushing to media.
const SD_SYNC_EVERY_ROWS: u32 = 25;
/// Minimum milliseconds between `sync_data()` calls (whichever comes first with row count).
const SD_SYNC_EVERY_MS: u64 = 1000;

/// Manages one active session WiGLE-format `.csv` file on SD.
pub(crate) struct Session {
    paths: Paths,
    file: Option<File>,
    wrote_header: bool,
    device_info: csv::DeviceInfo,
    header_tail_pos: u64,
    /// Bytes in the current open capture file (avoids per-row `metadata().len()`).
    file_len: u64,
    rows_since_sync: u32,
    last_sync_ms: u64,
}

impl Session {
    pub(crate) fn new() -> Self {
        Self {
            paths: Paths::new(),
            file: None,
            wrote_header: false,
            device_info: csv::DeviceInfo::default(),
            header_tail_pos: 0,
            file_len: 0,
            rows_since_sync: 0,
            last_sync_ms: 0,
        }
    }

    pub(crate) fn ensure_dirs(&self) -> IoResult<()> {
        // Worker thread only: touch VFS directly (never RPC back to sd-io).
        sd_vfs::create_dirs(&[self.paths.pending.clone(), self.paths.uploaded.clone()])
    }

    pub(crate) fn has_open_session(&self) -> bool {
        self.file.is_some()
    }

    pub(crate) fn open_session_when_ready(&mut self) -> IoResult<()> {
        if self.file.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "open_session_when_ready called while a session file is already open",
            ));
        }
        if !crate::time_sync::wall_clock_trusted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "wall clock not trusted (GPS time not applied)",
            ));
        }
        if !stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "SD card not mounted",
            ));
        }
        self.open_session_file()?;
        Ok(())
    }

    fn write_row_bytes_inner(&mut self, line: Vec<u8>) -> IoResult<()> {
        let line_len = line.len() as u64;
        loop {
            if self.file.is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "no WiGLE session file open (call open_session_when_ready first)",
                ));
            }
            let cur = self.file_len;
            if cur.saturating_add(line_len) <= WIGLE_CAPTURE_MAX_FILE_BYTES {
                break;
            }
            if cur > self.header_tail_pos {
                info!(
                    "storage: WiGLE session rotation at {} bytes (cap {} bytes)",
                    cur, WIGLE_CAPTURE_MAX_FILE_BYTES
                );
                self.close_current_session_file()?;
                self.open_session_file()?;
                continue;
            }
            warn!(
                "storage: single WiGLE row ({} bytes) exceeds cap boundary {}; writing anyway",
                line.len(),
                WIGLE_CAPTURE_MAX_FILE_BYTES
            );
            break;
        }

        let Some(f) = self.file.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no WiGLE session file open (call open_session_when_ready first)",
            ));
        };
        f.write_all(&line)?;
        self.file_len = self.file_len.saturating_add(line_len);
        self.rows_since_sync = self.rows_since_sync.saturating_add(1);
        Ok(())
    }

    pub(crate) fn write_row_bytes(&mut self, line: Vec<u8>) -> IoResult<()> {
        self.write_row_bytes_inner(line)
    }

    pub(crate) fn flush_pending_media(&mut self) -> IoResult<()> {
        if let Some(f) = self.file.as_mut() {
            flush_and_maybe_sync_media(f, &mut self.rows_since_sync, &mut self.last_sync_ms)?;
        }
        Ok(())
    }

    fn close_current_session_file(&mut self) -> IoResult<()> {
        if let Some(mut f) = self.file.take() {
            f.flush()?;
            f.sync_data()?;
        }
        self.file_len = 0;
        Ok(())
    }

    fn open_session_file(&mut self) -> IoResult<()> {
        self.ensure_dirs()?;
        let p = self.session_path()?;
        let mut f = sd_vfs::create_new_file(&p)?;
        csv::write_wigle_v16_headers(&mut f, &self.device_info)?;
        f.flush()?;
        f.sync_data()?;
        let header_tail_pos = f.metadata()?.len();
        self.rows_since_sync = 0;
        self.last_sync_ms = stats::now_ms();
        self.file = Some(f);
        self.wrote_header = true;
        self.header_tail_pos = header_tail_pos;
        self.file_len = header_tail_pos;
        stats::set_sd_session_created(true);
        info!("storage: session file created at {:?}", p);
        Ok(())
    }

    fn session_path(&self) -> IoResult<PathBuf> {
        let ts = crate::time_sync::now_utc_string()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "time not valid"))?;
        let name = ts.replace('-', "").replace(':', "").replace(' ', "-");
        Ok(self
            .paths
            .pending
            .join(format!("wd-{name}.{}", WIGLE_CAPTURE_EXT)))
    }
}

/// Manages one active session `.detectioncsv` file on SD for Flock detections.
pub(crate) struct DetectionSession {
    paths: DetectionPaths,
    file: Option<File>,
    device_info: csv::DeviceInfo,
    rows_since_sync: u32,
    last_sync_ms: u64,
    filename_prefix: &'static str,
}

impl DetectionSession {
    pub(crate) fn new() -> Self {
        Self::new_with_prefix("det")
    }

    pub(crate) fn new_with_prefix(filename_prefix: &'static str) -> Self {
        Self {
            paths: DetectionPaths::new(),
            file: None,
            device_info: csv::DeviceInfo::default(),
            rows_since_sync: 0,
            last_sync_ms: 0,
            filename_prefix,
        }
    }

    pub(crate) fn has_open_session(&self) -> bool {
        self.file.is_some()
    }

    pub(crate) fn open_session_when_ready(&mut self) -> IoResult<()> {
        if self.file.is_some() {
            return Ok(());
        }
        if !crate::time_sync::wall_clock_trusted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "wall clock not trusted (GPS time not applied)",
            ));
        }
        if !stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "SD card not mounted",
            ));
        }
        self.open_session_file()?;
        Ok(())
    }

    pub(crate) fn append_row_bytes(&mut self, line: Vec<u8>) -> IoResult<()> {
        let Some(f) = self.file.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no detection session file open (call open_session_when_ready first)",
            ));
        };
        f.write_all(&line)?;
        self.rows_since_sync = self.rows_since_sync.saturating_add(1);
        flush_and_maybe_sync_media(f, &mut self.rows_since_sync, &mut self.last_sync_ms)?;
        Ok(())
    }

    fn open_session_file(&mut self) -> IoResult<()> {
        sd_vfs::create_dirs(&[self.paths.pending.clone()])?;
        let p = self.session_path()?;
        let mut f = sd_vfs::create_new_file(&p)?;
        csv::write_detection_pre_header(&mut f, &self.device_info)?;
        csv::write_detection_header(&mut f)?;
        f.flush()?;
        f.sync_data()?;
        self.rows_since_sync = 0;
        self.last_sync_ms = stats::now_ms();
        self.file = Some(f);
        info!("storage: detection session file created at {:?}", p);
        Ok(())
    }

    fn session_path(&self) -> IoResult<PathBuf> {
        let ts = crate::time_sync::now_utc_string()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "time not valid"))?;
        let name = ts.replace('-', "").replace(':', "").replace(' ', "-");
        Ok(self
            .paths
            .pending
            .join(format!("{}-{name}.detectioncsv", self.filename_prefix)))
    }
}

/// Manages one active PACK `Axon-*.mdetectioncsv` session under `mobile_surveillance/detections/`.
pub(crate) struct MdetectionSession {
    paths: MdetectionPaths,
    file: Option<File>,
    device_info: csv::DeviceInfo,
    rows_since_sync: u32,
    last_sync_ms: u64,
}

impl MdetectionSession {
    pub(crate) fn new() -> Self {
        Self {
            paths: MdetectionPaths::new(),
            file: None,
            device_info: csv::DeviceInfo::default(),
            rows_since_sync: 0,
            last_sync_ms: 0,
        }
    }

    pub(crate) fn has_open_session(&self) -> bool {
        self.file.is_some()
    }

    pub(crate) fn open_session_when_ready(&mut self) -> IoResult<()> {
        if self.file.is_some() {
            return Ok(());
        }
        if !crate::time_sync::wall_clock_trusted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "wall clock not trusted (GPS time not applied)",
            ));
        }
        if !stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "SD card not mounted",
            ));
        }
        self.open_session_file()?;
        Ok(())
    }

    pub(crate) fn append_row_bytes(&mut self, line: Vec<u8>) -> IoResult<()> {
        let Some(f) = self.file.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no mdetection session file open (call open_session_when_ready first)",
            ));
        };
        f.write_all(&line)?;
        self.rows_since_sync = self.rows_since_sync.saturating_add(1);
        flush_and_maybe_sync_media(f, &mut self.rows_since_sync, &mut self.last_sync_ms)?;
        Ok(())
    }

    fn open_session_file(&mut self) -> IoResult<()> {
        sd_vfs::create_dirs(&[self.paths.pending.clone()])?;
        let p = self.session_path()?;
        let mut f = sd_vfs::create_new_file(&p)?;
        csv::write_mdetection_pre_header(&mut f, &self.device_info)?;
        csv::write_mdetection_header(&mut f)?;
        f.flush()?;
        f.sync_data()?;
        self.rows_since_sync = 0;
        self.last_sync_ms = stats::now_ms();
        self.file = Some(f);
        info!("storage: mdetection session file created at {:?}", p);
        Ok(())
    }

    fn session_path(&self) -> IoResult<PathBuf> {
        let ts = crate::time_sync::now_utc_string()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "time not valid"))?;
        let name = ts.replace('-', "").replace(':', "").replace(' ', "-");
        Ok(self
            .paths
            .pending
            .join(format!("Axon-{name}.mdetectioncsv")))
    }
}

/// Manages one active Remote ID `rid-*.PACKcsv` session under `remote_id/detections/`.
pub(crate) struct RemoteIdSession {
    paths: RemoteIdPaths,
    file: Option<File>,
    device_info: csv::DeviceInfo,
    rows_since_sync: u32,
    last_sync_ms: u64,
}

impl RemoteIdSession {
    pub(crate) fn new() -> Self {
        Self {
            paths: RemoteIdPaths::new(),
            file: None,
            device_info: csv::DeviceInfo::default(),
            rows_since_sync: 0,
            last_sync_ms: 0,
        }
    }

    pub(crate) fn has_open_session(&self) -> bool {
        self.file.is_some()
    }

    pub(crate) fn open_session_when_ready(&mut self) -> IoResult<()> {
        if self.file.is_some() {
            return Ok(());
        }
        if !crate::time_sync::wall_clock_trusted() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "wall clock not trusted (GPS time not applied)",
            ));
        }
        if !stats::SD_MOUNT_OK.load(std::sync::atomic::Ordering::Acquire) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "SD card not mounted",
            ));
        }
        self.open_session_file()?;
        Ok(())
    }

    pub(crate) fn append_row_bytes(&mut self, line: Vec<u8>) -> IoResult<()> {
        let Some(f) = self.file.as_mut() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "no remote id session file open (call open_session_when_ready first)",
            ));
        };
        f.write_all(&line)?;
        self.rows_since_sync = self.rows_since_sync.saturating_add(1);
        flush_and_maybe_sync_media(f, &mut self.rows_since_sync, &mut self.last_sync_ms)?;
        Ok(())
    }

    fn open_session_file(&mut self) -> IoResult<()> {
        sd_vfs::create_dirs(&[self.paths.pending.clone()])?;
        let p = self.session_path()?;
        let mut f = sd_vfs::create_new_file(&p)?;
        csv::write_pack_remote_id_pre_header(&mut f, &self.device_info)?;
        csv::write_pack_remote_id_header(&mut f)?;
        f.flush()?;
        f.sync_data()?;
        self.rows_since_sync = 0;
        self.last_sync_ms = stats::now_ms();
        self.file = Some(f);
        info!("storage: remote id session file created at {:?}", p);
        Ok(())
    }

    fn session_path(&self) -> IoResult<PathBuf> {
        let ts = crate::time_sync::now_utc_string()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "time not valid"))?;
        let name = ts.replace('-', "").replace(':', "").replace(' ', "-");
        Ok(self
            .paths
            .pending
            .join(format!("rid-{name}.{}", csv::PACKCSV_EXT)))
    }
}

fn flush_and_maybe_sync_media(
    f: &mut File,
    rows_since_sync: &mut u32,
    last_sync_ms: &mut u64,
) -> IoResult<()> {
    f.flush()?;
    let now = stats::now_ms();
    let elapsed = if *last_sync_ms == 0 {
        u64::MAX
    } else {
        now.saturating_sub(*last_sync_ms)
    };
    let need_sync = *rows_since_sync >= SD_SYNC_EVERY_ROWS || elapsed >= SD_SYNC_EVERY_MS;
    if need_sync {
        wardriver_shared::task_wdt::reset_if_subscribed();
        f.sync_data()?;
        *rows_since_sync = 0;
        *last_sync_ms = now;
    }
    Ok(())
}
