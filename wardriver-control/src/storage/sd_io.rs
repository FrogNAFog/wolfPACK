//! Single SD I/O worker: RPC channel (blocking callers) + async channel (hot path rows).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    sync_channel, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError,
};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use esp_idf_hal::delay::FreeRtos;
use log::{info, warn};

use super::sd_sessions::{DetectionSession, MdetectionSession, RemoteIdSession, Session};
use super::sd_vfs::{self, DiagLogWriter, ReconWriter};
use super::CleanupResult;
use wardriver_shared::spiram_thread;
use wardriver_stats as stats;

/// Bounded RPC queue depth (httpd / boot / upload thread callers).
pub const SD_IO_RPC_CAPACITY: usize = 8;
/// Default [`SdIoHandle::submit_and_wait`] timeout.
pub const SD_IO_RPC_WAIT_MS: u64 = 5000;
/// Upload / boot paths that must wait behind an in-flight WiGLE flush.
pub const SD_IO_RPC_UPLOAD_WAIT_MS: u64 = 15_000;

/// Bounded queue capacity for async SD writer messages (row batches, diag, recon chunks).
pub const SD_WRITER_QUEUE_CAPACITY: usize = 512;
static SD_WRITER_QUEUE_CAPACITY_RUNTIME: OnceLock<usize> = OnceLock::new();

pub fn configure_sd_writer_queue_capacity(cap: usize) {
    let _ = SD_WRITER_QUEUE_CAPACITY_RUNTIME.set(cap.max(64));
}

pub fn sd_writer_queue_capacity() -> usize {
    SD_WRITER_QUEUE_CAPACITY_RUNTIME
        .get()
        .copied()
        .unwrap_or(SD_WRITER_QUEUE_CAPACITY)
}

static SD_IO: OnceLock<Arc<SdIoHandle>> = OnceLock::new();
static SD_IO_WORKER_READY: AtomicBool = AtomicBool::new(false);

pub(crate) fn signal_sd_io_worker_ready() {
    SD_IO_WORKER_READY.store(true, Ordering::SeqCst);
}

/// Global SD I/O handle for hooks that cannot take `&SdIoHandle` (e.g. GPS recovery logger).
pub fn register_sd_io_handle(handle: Arc<SdIoHandle>) {
    let _ = SD_IO.set(handle);
}

pub fn sd_io_handle() -> Option<&'static Arc<SdIoHandle>> {
    SD_IO.get()
}

/// Synchronous SD operations submitted to the worker (drained before each async message).
#[derive(Debug)]
pub enum SdIoRpcOp {
    ReadText(PathBuf),
    WriteAtomicText {
        path: PathBuf,
        body: Vec<u8>,
    },
    ReadDir(PathBuf),
    UploadReadOpen(PathBuf),
    UploadReadSeek(u64),
    UploadReadChunk(usize),
    UploadReadClose,
    FileMetadata(PathBuf),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Remove(PathBuf),
    CreateDirs(Vec<PathBuf>),
    CleanupHeaderOnlyWigle,
    CountPendingCsv,
    ListPendingCsv,
    ReconOpen {
        pcap_path: PathBuf,
        meta_path: PathBuf,
        initial: Vec<u8>,
    },
    ReconClose {
        meta_json: String,
    },
    AppendDiagLine(String),
    WriteEmptyFile(PathBuf),
}

/// Reply payload for [`SdIoRpcOp`].
#[derive(Debug)]
pub enum SdIoRpcResult {
    Text(Result<Option<String>, std::io::Error>),
    Bytes(Result<Vec<u8>, std::io::Error>),
    Unit(Result<(), std::io::Error>),
    U64(Result<u64, std::io::Error>),
    U32(u32),
    Paths(Result<Vec<PathBuf>, std::io::Error>),
    Cleanup(CleanupResult),
}

/// Per-request reply slot: worker stores result; caller polls (no pthread condvar wait).
struct RpcReplySlot {
    inner: Mutex<Option<SdIoRpcResult>>,
}

impl RpcReplySlot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
        })
    }

    fn wait(self: &Arc<Self>, timeout: Duration) -> Result<SdIoRpcResult, RecvTimeoutError> {
        let deadline_ms = wardriver_stats::now_ms() + timeout.as_millis() as u64;
        loop {
            wardriver_shared::task_wdt::reset_if_subscribed();
            if let Ok(mut g) = self.inner.lock() {
                if let Some(r) = g.take() {
                    return Ok(r);
                }
            }
            if wardriver_stats::now_ms() >= deadline_ms {
                return Err(RecvTimeoutError::Timeout);
            }
            wardriver_shared::task_wdt::sleep_feeding_wdt(Duration::from_millis(10));
        }
    }

    fn complete(&self, result: SdIoRpcResult) {
        if let Ok(mut g) = self.inner.lock() {
            *g = Some(result);
        }
    }
}

struct SdIoRpcRequest {
    op: SdIoRpcOp,
    reply: Arc<RpcReplySlot>,
}

/// Async fire-and-forget messages (hot path).
pub enum SdIoAsync {
    OpenWigleSession,
    WigleRows(Vec<Vec<u8>>),
    DetectionRow(Vec<u8>),
    MdetectionRow(Vec<u8>),
    RemoteIdRow(Vec<u8>),
    DiagLogLine(String),
    ReconPcapChunk(Vec<u8>),
}

/// Handle returned by [`spawn_sd_io_worker`]. Keep alive for the entire control main loop.
#[derive(Clone)]
pub struct SdIoHandle {
    rpc_tx: SyncSender<SdIoRpcRequest>,
    async_tx: SyncSender<SdIoAsync>,
}

pub type SdWriterHandle = SdIoHandle;

impl SdIoHandle {
    /// Blocking RPC to the SD worker (default 500 ms timeout).
    pub fn submit_and_wait(&self, op: SdIoRpcOp) -> SdIoRpcResult {
        self.submit_and_wait_timeout(op, Duration::from_millis(SD_IO_RPC_WAIT_MS))
    }

    pub fn submit_and_wait_timeout(&self, op: SdIoRpcOp, timeout: Duration) -> SdIoRpcResult {
        let t0 = Instant::now();
        let reply = RpcReplySlot::new();
        let req = SdIoRpcRequest {
            op,
            reply: Arc::clone(&reply),
        };
        let worker_wait = Duration::from_millis(200);
        let worker_deadline = Instant::now() + worker_wait;
        while !SD_IO_WORKER_READY.load(Ordering::SeqCst) {
            wardriver_shared::task_wdt::reset_if_subscribed();
            if Instant::now() >= worker_deadline {
                break;
            }
            wardriver_shared::task_wdt::sleep_feeding_wdt(Duration::from_millis(5));
        }
        match self.rpc_tx.try_send(req) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                stats::note_sd_io_rpc_timeout();
                return SdIoRpcResult::Unit(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "sd-io rpc queue full",
                )));
            }
            Err(TrySendError::Disconnected(_)) => {
                return SdIoRpcResult::Unit(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "sd-io rpc queue disconnected",
                )));
            }
        }
        let result = match reply.wait(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => {
                stats::note_sd_io_rpc_timeout();
                return SdIoRpcResult::Unit(Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "sd-io rpc timed out",
                )));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return SdIoRpcResult::Unit(Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "sd-io rpc reply disconnected",
                )));
            }
        };
        let ms = t0.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        stats::SD_IO_LAST_RPC_MS.store(ms, Ordering::Relaxed);
        result
    }

    fn try_send_async(&self, msg: SdIoAsync) -> Result<(), TrySendError<SdIoAsync>> {
        self.async_tx.try_send(msg)
    }

    fn on_queue_full_dropped_rows(&self, n: usize) {
        stats::SD_WRITER_QUEUE_FULL.fetch_add(1, Ordering::Relaxed);
        stats::add_rows_dropped_sd(n);
    }

    pub fn try_enqueue_wigle_rows(&self, lines: Vec<Vec<u8>>) -> bool {
        let n = lines.len();
        if n == 0 {
            return true;
        }
        match self.try_send_async(SdIoAsync::WigleRows(lines)) {
            Ok(()) => {
                stats::queue_inflight_inc(
                    &stats::SD_WRITER_QUEUE_INFLIGHT,
                    &stats::SD_WRITER_QUEUE_HWM,
                );
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.on_queue_full_dropped_rows(n);
                false
            }
        }
    }

    pub fn try_enqueue_detection_row(&self, line: Vec<u8>) -> bool {
        self.try_enqueue_single(SdIoAsync::DetectionRow(line))
    }

    pub fn try_enqueue_mdetection_row(&self, line: Vec<u8>) -> bool {
        self.try_enqueue_single(SdIoAsync::MdetectionRow(line))
    }

    pub fn try_enqueue_remote_id_row(&self, line: Vec<u8>) -> bool {
        self.try_enqueue_single(SdIoAsync::RemoteIdRow(line))
    }

    pub fn try_enqueue_diag_line(&self, line: String) -> bool {
        self.try_enqueue_single(SdIoAsync::DiagLogLine(line))
    }

    pub fn try_enqueue_recon_chunk(&self, data: Vec<u8>) -> bool {
        if data.is_empty() {
            return true;
        }
        match self.try_send_async(SdIoAsync::ReconPcapChunk(data)) {
            Ok(()) => {
                stats::queue_inflight_inc(
                    &stats::SD_WRITER_QUEUE_INFLIGHT,
                    &stats::SD_WRITER_QUEUE_HWM,
                );
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                stats::RECON_SD_CHUNK_DROPS.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    fn try_enqueue_single(&self, msg: SdIoAsync) -> bool {
        match self.try_send_async(msg) {
            Ok(()) => {
                stats::queue_inflight_inc(
                    &stats::SD_WRITER_QUEUE_INFLIGHT,
                    &stats::SD_WRITER_QUEUE_HWM,
                );
                true
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.on_queue_full_dropped_rows(1);
                false
            }
        }
    }

    pub fn try_request_open_session(&self) -> bool {
        stats::sd_eager_open_requested();
        match self.try_send_async(SdIoAsync::OpenWigleSession) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                stats::sd_eager_open_request_dropped();
                false
            }
        }
    }

    // ---- RPC convenience wrappers ----------------------------------------------------------

    pub fn rpc_read_text(&self, path: PathBuf) -> Result<Option<String>, std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::ReadText(path)) {
            SdIoRpcResult::Text(r) => r,
            other => rpc_transport_err(other, "read text"),
        }
    }

    pub fn rpc_write_atomic_text(
        &self,
        path: PathBuf,
        body: impl Into<Vec<u8>>,
    ) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::WriteAtomicText {
            path,
            body: body.into(),
        }) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "write atomic text"),
        }
    }

    pub fn rpc_read_dir(&self, dir: PathBuf) -> Result<Vec<PathBuf>, std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::ReadDir(dir)) {
            SdIoRpcResult::Paths(r) => r,
            other => rpc_transport_err(other, "read dir"),
        }
    }

    pub fn rpc_upload_read_open(&self, path: PathBuf) -> Result<(), std::io::Error> {
        let timeout = Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS);
        match self.submit_and_wait_timeout(SdIoRpcOp::UploadReadOpen(path), timeout) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "upload read open"),
        }
    }

    pub fn rpc_upload_read_seek(&self, offset: u64) -> Result<(), std::io::Error> {
        let timeout = Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS);
        match self.submit_and_wait_timeout(SdIoRpcOp::UploadReadSeek(offset), timeout) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "upload read seek"),
        }
    }

    pub fn rpc_upload_read_chunk(&self, len: usize) -> Result<Vec<u8>, std::io::Error> {
        let timeout = Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS);
        match self.submit_and_wait_timeout(SdIoRpcOp::UploadReadChunk(len), timeout) {
            SdIoRpcResult::Bytes(r) => r,
            other => rpc_transport_err(other, "upload read chunk"),
        }
    }

    pub fn rpc_upload_read_close(&self) {
        let timeout = Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS);
        let _ = self.submit_and_wait_timeout(SdIoRpcOp::UploadReadClose, timeout);
    }

    pub fn rpc_file_len(&self, path: PathBuf) -> Result<u64, std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::FileMetadata(path)) {
            SdIoRpcResult::U64(r) => r,
            other => rpc_transport_err(other, "file metadata"),
        }
    }

    pub fn rpc_rename(&self, from: PathBuf, to: PathBuf) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::Rename { from, to }) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "rename"),
        }
    }

    pub fn rpc_remove(&self, path: PathBuf) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::Remove(path)) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "remove"),
        }
    }

    pub fn rpc_count_pending_csv(&self) -> u32 {
        match self.submit_and_wait_timeout(
            SdIoRpcOp::CountPendingCsv,
            Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS),
        ) {
            SdIoRpcResult::U32(n) => n,
            _ => stats::upload_pending_csv_count(),
        }
    }

    pub fn rpc_list_pending_csv(&self) -> Result<Vec<PathBuf>, std::io::Error> {
        let timeout = Duration::from_millis(SD_IO_RPC_UPLOAD_WAIT_MS);
        match self.submit_and_wait_timeout(SdIoRpcOp::ListPendingCsv, timeout) {
            SdIoRpcResult::Paths(r) => r,
            other => rpc_transport_err(other, "list pending csv"),
        }
    }

    pub fn rpc_recon_open(
        &self,
        pcap_path: PathBuf,
        meta_path: PathBuf,
        initial: Vec<u8>,
    ) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::ReconOpen {
            pcap_path,
            meta_path,
            initial,
        }) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "recon open"),
        }
    }

    pub fn rpc_recon_close(&self, meta_json: String) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::ReconClose { meta_json }) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "recon close"),
        }
    }

    pub fn rpc_append_diag_line(&self, line: &str) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::AppendDiagLine(line.to_string())) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "append diag line"),
        }
    }

    pub fn rpc_write_empty_file(&self, path: PathBuf) -> Result<(), std::io::Error> {
        match self.submit_and_wait(SdIoRpcOp::WriteEmptyFile(path)) {
            SdIoRpcResult::Unit(r) => r,
            other => rpc_transport_err(other, "write empty file"),
        }
    }
}

fn rpc_transport_err<T>(other: SdIoRpcResult, op: &'static str) -> Result<T, std::io::Error> {
    if let SdIoRpcResult::Unit(Err(e)) = other {
        return Err(e);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("sd-io rpc {op}: unexpected reply"),
    ))
}

/// Spawn the unified SD I/O thread (WiGLE, detection, mdetection, diag log, RPC).
pub fn spawn_sd_io_worker() -> SdIoHandle {
    let async_cap = sd_writer_queue_capacity();
    let (async_tx, async_rx) = sync_channel::<SdIoAsync>(async_cap);
    let (rpc_tx, rpc_rx) = sync_channel::<SdIoRpcRequest>(SD_IO_RPC_CAPACITY);
    spiram_thread::spawn_named("sd-io", spiram_thread::SD_IO_STACK_BYTES, move || {
        sd_io_worker_task(rpc_rx, async_rx, async_cap);
    })
    .expect("spawn sd-io thread (128 KiB stack)");
    SdIoHandle { rpc_tx, async_tx }
}

pub fn spawn_sd_writer() -> SdIoHandle {
    spawn_sd_io_worker()
}

fn append_diag_line_worker(state: &mut WorkerState, line: &str) -> Result<(), std::io::Error> {
    if state.diag_log.is_none() {
        state.diag_log = DiagLogWriter::open().ok();
    }
    if let Some(w) = state.diag_log.as_mut() {
        w.append_line(line)
    } else {
        sd_vfs::write_diag_log_line(line)
    }
}

/// Session-file ops shared by the detection / mdetection / remote-id sessions, so
/// `handle_async` can collapse its three `*Row` arms into one [`append_session_row`].
trait SessionRow {
    fn has_open_session(&self) -> bool;
    fn open_session_when_ready(&mut self) -> std::io::Result<()>;
    fn append_row_bytes(&mut self, line: Vec<u8>) -> std::io::Result<()>;
}

impl SessionRow for DetectionSession {
    fn has_open_session(&self) -> bool {
        DetectionSession::has_open_session(self)
    }
    fn open_session_when_ready(&mut self) -> std::io::Result<()> {
        DetectionSession::open_session_when_ready(self)
    }
    fn append_row_bytes(&mut self, line: Vec<u8>) -> std::io::Result<()> {
        DetectionSession::append_row_bytes(self, line)
    }
}

impl SessionRow for MdetectionSession {
    fn has_open_session(&self) -> bool {
        MdetectionSession::has_open_session(self)
    }
    fn open_session_when_ready(&mut self) -> std::io::Result<()> {
        MdetectionSession::open_session_when_ready(self)
    }
    fn append_row_bytes(&mut self, line: Vec<u8>) -> std::io::Result<()> {
        MdetectionSession::append_row_bytes(self, line)
    }
}

impl SessionRow for RemoteIdSession {
    fn has_open_session(&self) -> bool {
        RemoteIdSession::has_open_session(self)
    }
    fn open_session_when_ready(&mut self) -> std::io::Result<()> {
        RemoteIdSession::open_session_when_ready(self)
    }
    fn append_row_bytes(&mut self, line: Vec<u8>) -> std::io::Result<()> {
        RemoteIdSession::append_row_bytes(self, line)
    }
}

/// One detection/session row for a detection / mdetection / remote-id session: open the
/// session if needed, append the row, and record the remote-id success counter.
/// The two `drain_rpc_pending` calls around it stay in `handle_async` (they need
/// `&mut state`, which this helper must not hold).
fn append_session_row(
    session: &mut dyn SessionRow,
    line: Vec<u8>,
    label: &str,
    count_remote_id_success: bool,
) {
    if !session.has_open_session() {
        if let Err(e) = session.open_session_when_ready() {
            warn!("storage/sd-io: {label} session open: {e:#}");
            stats::add_rows_dropped_sd(1);
            return;
        }
    }
    if let Err(e) = session.append_row_bytes(line) {
        warn!("storage/sd-io: {label} append: {e:#}");
        stats::add_rows_dropped_sd(1);
    } else if count_remote_id_success {
        stats::add_remote_id_rows_written(1);
    }
}

struct WorkerState {
    wigle: Session,
    detection: DetectionSession,
    mdetection: MdetectionSession,
    remote_id: RemoteIdSession,
    recon: Option<ReconWriter>,
    upload_read: Option<sd_vfs::UploadReadFile>,
    diag_log: Option<DiagLogWriter>,
}

fn sd_io_worker_task(
    rpc_rx: Receiver<SdIoRpcRequest>,
    async_rx: Receiver<SdIoAsync>,
    async_cap: usize,
) {
    let mut state = WorkerState {
        wigle: Session::new(),
        detection: DetectionSession::new(),
        mdetection: MdetectionSession::new(),
        remote_id: RemoteIdSession::new(),
        recon: None,
        upload_read: None,
        diag_log: None,
    };
    wardriver_shared::task_wdt::register_current_task_for_watchdog();
    signal_sd_io_worker_ready();
    info!(
        "storage/sd-io: thread started (async cap {}, rpc cap {})",
        async_cap, SD_IO_RPC_CAPACITY
    );
    loop {
        wardriver_shared::task_wdt::reset_if_subscribed();

        let mut did_work = false;
        loop {
            let mut progressed = false;
            while let Ok(req) = rpc_rx.try_recv() {
                progressed = true;
                did_work = true;
                dispatch_rpc(&mut state, req, &rpc_rx);
                wardriver_shared::task_wdt::reset_if_subscribed();
            }
            if let Ok(msg) = async_rx.try_recv() {
                progressed = true;
                did_work = true;
                handle_async(&mut state, msg, &rpc_rx);
                wardriver_shared::task_wdt::reset_if_subscribed();
            }
            if !progressed {
                break;
            }
        }

        match rpc_rx.try_recv() {
            Ok(req) => {
                dispatch_rpc(&mut state, req, &rpc_rx);
                wardriver_shared::task_wdt::reset_if_subscribed();
                continue;
            }
            Err(TryRecvError::Disconnected) => {
                info!("storage/sd-io: RPC channel closed");
                return;
            }
            Err(TryRecvError::Empty) => {}
        }

        if !did_work {
            FreeRtos::delay_ms(5);
        }
    }
}

/// Service any RPC queued while a long async or RPC handler runs.
fn drain_rpc_pending(rpc_rx: &Receiver<SdIoRpcRequest>, state: &mut WorkerState) {
    while let Ok(req) = rpc_rx.try_recv() {
        dispatch_rpc(state, req, rpc_rx);
        wardriver_shared::task_wdt::reset_if_subscribed();
    }
}

fn dispatch_rpc(state: &mut WorkerState, req: SdIoRpcRequest, rpc_rx: &Receiver<SdIoRpcRequest>) {
    let result = match req.op {
        SdIoRpcOp::ReadText(path) => SdIoRpcResult::Text(sd_vfs::read_text(&path)),
        SdIoRpcOp::WriteAtomicText { path, body } => {
            SdIoRpcResult::Unit(sd_vfs::write_atomic_text(&path, &body))
        }
        SdIoRpcOp::ReadDir(dir) => SdIoRpcResult::Paths(sd_vfs::read_dir_paths(&dir)),
        SdIoRpcOp::UploadReadOpen(path) => {
            state.upload_read = None;
            let r = sd_vfs::UploadReadFile::open(&path).map(|f| {
                state.upload_read = Some(f);
            });
            SdIoRpcResult::Unit(r)
        }
        SdIoRpcOp::UploadReadSeek(offset) => {
            let r = match state.upload_read.as_mut() {
                Some(f) => f.seek(offset),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "upload read session not open",
                )),
            };
            SdIoRpcResult::Unit(r)
        }
        SdIoRpcOp::UploadReadChunk(len) => {
            let r = match state.upload_read.as_mut() {
                Some(f) => f.read_chunk(len),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "upload read session not open",
                )),
            };
            SdIoRpcResult::Bytes(r)
        }
        SdIoRpcOp::UploadReadClose => {
            state.upload_read = None;
            SdIoRpcResult::Unit(Ok(()))
        }
        SdIoRpcOp::FileMetadata(path) => SdIoRpcResult::U64(sd_vfs::file_len(&path)),
        SdIoRpcOp::Rename { from, to } => SdIoRpcResult::Unit(sd_vfs::rename_path(&from, &to)),
        SdIoRpcOp::Remove(path) => SdIoRpcResult::Unit(sd_vfs::remove_path(&path)),
        SdIoRpcOp::CreateDirs(paths) => SdIoRpcResult::Unit(sd_vfs::create_dirs(&paths)),
        SdIoRpcOp::CleanupHeaderOnlyWigle => {
            let out = sd_vfs::cleanup_header_only_wiglecsv_files_yield(|| {
                drain_rpc_pending(rpc_rx, state);
            });
            SdIoRpcResult::Cleanup(out)
        }
        SdIoRpcOp::CountPendingCsv => {
            let n = sd_vfs::count_pending_wigle_csvs();
            stats::set_upload_pending_csv_count(n);
            SdIoRpcResult::U32(n)
        }
        SdIoRpcOp::ListPendingCsv => SdIoRpcResult::Paths(sd_vfs::pending_csv_abs_names()),
        SdIoRpcOp::ReconOpen {
            pcap_path,
            meta_path,
            initial,
        } => {
            let r = sd_vfs::recon_open(&pcap_path, &meta_path, &initial).map(|w| {
                state.recon = Some(w);
            });
            SdIoRpcResult::Unit(r)
        }
        SdIoRpcOp::ReconClose { meta_json } => {
            let r = match state.recon.take() {
                Some(w) => sd_vfs::recon_close(w, &meta_json),
                None => Ok(()),
            };
            SdIoRpcResult::Unit(r)
        }
        SdIoRpcOp::AppendDiagLine(line) => match append_diag_line_worker(state, &line) {
            Ok(()) => SdIoRpcResult::Unit(Ok(())),
            Err(e) => {
                stats::diag_log_line_err();
                SdIoRpcResult::Unit(Err(e))
            }
        },
        SdIoRpcOp::WriteEmptyFile(path) => SdIoRpcResult::Unit(sd_vfs::write_empty_file(&path)),
    };
    req.reply.complete(result);
}

fn handle_async(state: &mut WorkerState, msg: SdIoAsync, rpc_rx: &Receiver<SdIoRpcRequest>) {
    match msg {
        SdIoAsync::OpenWigleSession => {
            if state.wigle.has_open_session() {
                return;
            }
            drain_rpc_pending(rpc_rx, state);
            match state.wigle.open_session_when_ready() {
                Ok(()) => stats::sd_eager_open_succeeded(),
                Err(e) => {
                    warn!("storage/sd-io: eager WiGLE session open: {e:#}");
                    stats::sd_eager_open_failed();
                }
            }
            drain_rpc_pending(rpc_rx, state);
        }
        SdIoAsync::WigleRows(lines) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            if lines.is_empty() {
                return;
            }
            drain_rpc_pending(rpc_rx, state);
            if !state.wigle.has_open_session() {
                if let Err(e) = state.wigle.open_session_when_ready() {
                    warn!("storage/sd-io: WiGLE session open: {e:#}");
                    stats::add_rows_dropped_sd(lines.len());
                    return;
                }
                drain_rpc_pending(rpc_rx, state);
            }
            let t0 = Instant::now();
            let mut written = 0usize;
            for line in lines {
                drain_rpc_pending(rpc_rx, state);
                match state.wigle.write_row_bytes(line) {
                    Ok(()) => written = written.saturating_add(1),
                    Err(e) => {
                        warn!("storage/sd-io: WiGLE write_row_bytes: {e:#}");
                        stats::add_rows_dropped_sd(1);
                    }
                }
            }
            if written > 0 {
                drain_rpc_pending(rpc_rx, state);
                if let Err(e) = state.wigle.flush_pending_media() {
                    warn!("storage/sd-io: WiGLE flush: {e:#}");
                    stats::add_rows_dropped_sd(written);
                } else {
                    let us = t0.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
                    stats::record_sd_append_us(us);
                    stats::add_rows_written(written);
                }
                drain_rpc_pending(rpc_rx, state);
            }
        }
        SdIoAsync::DetectionRow(line) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            drain_rpc_pending(rpc_rx, state);
            append_session_row(&mut state.detection, line, "detection", false);
            drain_rpc_pending(rpc_rx, state);
        }
        SdIoAsync::MdetectionRow(line) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            drain_rpc_pending(rpc_rx, state);
            append_session_row(&mut state.mdetection, line, "mdetection", false);
            drain_rpc_pending(rpc_rx, state);
        }
        SdIoAsync::RemoteIdRow(line) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            drain_rpc_pending(rpc_rx, state);
            append_session_row(&mut state.remote_id, line, "remote id", true);
            drain_rpc_pending(rpc_rx, state);
        }
        SdIoAsync::DiagLogLine(line) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            drain_rpc_pending(rpc_rx, state);
            if let Err(e) = append_diag_line_worker(state, &line) {
                warn!("storage/sd-io: diag log: {e:#}");
                stats::diag_log_line_err();
            }
            drain_rpc_pending(rpc_rx, state);
        }
        SdIoAsync::ReconPcapChunk(data) => {
            stats::queue_inflight_dec(&stats::SD_WRITER_QUEUE_INFLIGHT);
            drain_rpc_pending(rpc_rx, state);
            match state.recon.as_mut() {
                Some(w) => {
                    if let Err(e) = sd_vfs::recon_append(w, &data) {
                        warn!("storage/sd-io: recon append: {e:#}");
                        stats::RECON_SD_CHUNK_DROPS.fetch_add(1, Ordering::Relaxed);
                    }
                }
                None => {
                    stats::RECON_SD_CHUNK_DROPS.fetch_add(1, Ordering::Relaxed);
                }
            }
            drain_rpc_pending(rpc_rx, state);
        }
    }
}
