//! Bounded MPSC channel with a preallocated ring buffer in SPIRAM (internal fallback).
//!
//! Matches the `try_send` / `try_recv` / `recv_timeout` surface used by ESP-NOW RX ingest.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::spiram_heap;

pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

pub enum TryRecvError {
    Empty,
    Disconnected,
}

pub enum RecvTimeoutError {
    Timeout,
    Disconnected,
}

struct Inner<T> {
    buf: *mut T,
    cap: usize,
    state: Mutex<QueueState>,
    cvar: Condvar,
    receivers: AtomicUsize,
    senders: AtomicUsize,
}

struct QueueState {
    head: usize,
    len: usize,
}

unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        if self.buf.is_null() {
            return;
        }
        if let Ok(st) = self.state.lock() {
            let mut idx = st.head;
            for _ in 0..st.len {
                unsafe {
                    std::ptr::drop_in_place(self.buf.add(idx));
                }
                idx = (idx + 1) % self.cap;
            }
        }
        spiram_heap::free_bytes(self.buf.cast());
    }
}

impl<T> Inner<T> {
    fn slot_ptr(&self, index: usize) -> *mut T {
        unsafe { self.buf.add(index) }
    }
}

pub struct SyncSender<T> {
    inner: Arc<Inner<T>>,
}

pub struct Receiver<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        self.inner.senders.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> Drop for SyncSender<T> {
    fn drop(&mut self) {
        if self.inner.senders.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.cvar.notify_all();
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if self.inner.receivers.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.cvar.notify_all();
        }
    }
}

fn alloc_ring<T>(cap: usize) -> *mut T {
    let bytes = cap
        .checked_mul(std::mem::size_of::<T>())
        .expect("spiram_queue: capacity overflow");
    spiram_heap::alloc_bytes(bytes).cast()
}

/// Preallocate a bounded channel (`cap` slots). Buffer lives in SPIRAM when present.
pub fn bounded_channel<T: Send>(cap: usize) -> (SyncSender<T>, Receiver<T>) {
    let cap = cap.max(1);
    let buf = alloc_ring::<T>(cap);
    let inner = Arc::new(Inner {
        buf,
        cap,
        state: Mutex::new(QueueState { head: 0, len: 0 }),
        cvar: Condvar::new(),
        receivers: AtomicUsize::new(1),
        senders: AtomicUsize::new(1),
    });
    (
        SyncSender {
            inner: Arc::clone(&inner),
        },
        Receiver { inner },
    )
}

impl<T> SyncSender<T> {
    pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
        let mut st = match self.inner.state.lock() {
            Ok(g) => g,
            Err(_) => return Err(TrySendError::Disconnected(value)),
        };
        if self.inner.receivers.load(Ordering::Acquire) == 0 {
            return Err(TrySendError::Disconnected(value));
        }
        if st.len >= self.inner.cap {
            return Err(TrySendError::Full(value));
        }
        let write_idx = (st.head + st.len) % self.inner.cap;
        unsafe {
            std::ptr::write(self.inner.slot_ptr(write_idx), value);
        }
        st.len += 1;
        self.inner.cvar.notify_one();
        Ok(())
    }
}

impl<T> Receiver<T> {
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        let mut st = match self.inner.state.lock() {
            Ok(g) => g,
            Err(_) => return Err(TryRecvError::Disconnected),
        };
        if st.len == 0 {
            if self.inner.senders.load(Ordering::Acquire) == 0 {
                return Err(TryRecvError::Disconnected);
            }
            return Err(TryRecvError::Empty);
        }
        let frame = unsafe { std::ptr::read(self.inner.slot_ptr(st.head)) };
        st.head = (st.head + 1) % self.inner.cap;
        st.len -= 1;
        Ok(frame)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut st = self
            .inner
            .state
            .lock()
            .map_err(|_| RecvTimeoutError::Disconnected)?;
        loop {
            if st.len > 0 {
                let frame = unsafe { std::ptr::read(self.inner.slot_ptr(st.head)) };
                st.head = (st.head + 1) % self.inner.cap;
                st.len -= 1;
                return Ok(frame);
            }
            if self.inner.senders.load(Ordering::Acquire) == 0 {
                return Err(RecvTimeoutError::Disconnected);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RecvTimeoutError::Timeout);
            }
            let wait = deadline - now;
            st = self
                .inner
                .cvar
                .wait_timeout(st, wait)
                .map_err(|_| RecvTimeoutError::Disconnected)?
                .0;
        }
    }
}
