//! PSRAM ring buffer for PackLink frames when the phone SoftAP link is down.
//!
//! Volatile across reset (no flash partition). Sized for ~5–10 s of wardriving
//! with a full roster (~48 sniffers) at typical observation_bulk rates.

use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use log::{info, warn};
use wardriver_shared::spiram_heap;

/// ~1 MiB: ~48 sniffers × ~1.5 KiB peak PackLink frame × several bursts in 10 s.
pub const DEFAULT_SPOOL_CAPACITY: u32 = 1024 * 1024;
const RECORD_LEN_BYTES: u32 = 4;

struct SpoolMeta {
    head: u32,
    tail: u32,
    stored_bytes: u32,
    payload_bytes: u32,
    tail_epoch: u32,
}

struct SpoolBacking {
    buf: Vec<u8>,
    capacity: u32,
    meta: SpoolMeta,
}

static SPOOL: OnceLock<Mutex<SpoolBacking>> = OnceLock::new();

/// Initialize PSRAM-backed spool once.
pub fn init(capacity: u32) -> Result<()> {
    if SPOOL.get().is_some() {
        return Ok(());
    }
    let capacity = capacity.max(4096);
    let cap = capacity as usize;
    let mut buf = spiram_heap::vec_with_capacity::<u8>(cap);
    // SAFETY: ring accesses stay within [0, cap) through write_at/read_at.
    unsafe {
        buf.set_len(cap);
    }
    let _ = SPOOL.set(Mutex::new(SpoolBacking {
        buf,
        capacity,
        meta: SpoolMeta {
            head: 0,
            tail: 0,
            stored_bytes: 0,
            payload_bytes: 0,
            tail_epoch: 0,
        },
    }));
    info!("gateway spool: PSRAM cap={capacity}");
    Ok(())
}

fn with_spool_mut<F, R>(f: F) -> Result<R>
where
    F: FnOnce(&mut SpoolBacking) -> Result<R>,
{
    let guard = SPOOL
        .get()
        .ok_or_else(|| anyhow!("spool not initialized"))?;
    let mut spool = guard.lock().map_err(|_| anyhow!("spool lock"))?;
    f(&mut spool)
}

/// Append one complete PackLink frame, dropping oldest complete frames on overflow.
pub fn append(frame: &[u8]) -> Result<()> {
    if frame.is_empty() {
        return Ok(());
    }
    with_spool_mut(|spool| {
        let payload_len =
            u32::try_from(frame.len()).map_err(|_| anyhow!("spool frame length overflow"))?;
        let record_len = payload_len
            .checked_add(RECORD_LEN_BYTES)
            .ok_or_else(|| anyhow!("spool record length overflow"))?;
        let cap = spool.capacity;
        if record_len > cap {
            return Err(anyhow!(
                "PackLink frame too large for spool: payload={payload_len} capacity={cap}"
            ));
        }
        while spool.meta.stored_bytes.saturating_add(record_len) > cap {
            let dropped = drop_oldest_record(spool)?;
            warn!("gateway spool: dropped oldest PackLink record ({dropped} payload bytes)");
        }
        write_at(spool, spool.meta.head, &payload_len.to_le_bytes())?;
        spool.meta.head = (spool.meta.head + RECORD_LEN_BYTES) % cap;
        write_at(spool, spool.meta.head, frame)?;
        spool.meta.head = (spool.meta.head + payload_len) % cap;
        spool.meta.stored_bytes += record_len;
        spool.meta.payload_bytes += payload_len;
        Ok(())
    })
}

fn drop_oldest_record(spool: &mut SpoolBacking) -> Result<u32> {
    if spool.meta.stored_bytes < RECORD_LEN_BYTES {
        return Err(anyhow!("spool record metadata corrupt"));
    }
    let payload_len = read_u32_at(spool, spool.meta.tail)?;
    let record_len = payload_len
        .checked_add(RECORD_LEN_BYTES)
        .ok_or_else(|| anyhow!("spool record length corrupt"))?;
    if record_len > spool.meta.stored_bytes || payload_len > spool.meta.payload_bytes {
        return Err(anyhow!("spool record exceeds stored bytes"));
    }
    spool.meta.tail = (spool.meta.tail + record_len) % spool.capacity;
    spool.meta.stored_bytes -= record_len;
    spool.meta.payload_bytes -= payload_len;
    spool.meta.tail_epoch = spool.meta.tail_epoch.wrapping_add(1);
    if spool.meta.stored_bytes == 0 {
        spool.meta.head = spool.meta.tail;
    }
    Ok(payload_len)
}

fn write_at(spool: &mut SpoolBacking, offset: u32, data: &[u8]) -> Result<()> {
    let cap = spool.capacity;
    let mut off = offset % cap;
    let mut remaining = data;
    while !remaining.is_empty() {
        let until_wrap = (cap - off) as usize;
        let chunk = remaining.len().min(until_wrap);
        let start = off as usize;
        spool.buf[start..start + chunk].copy_from_slice(&remaining[..chunk]);
        off = (off + chunk as u32) % cap;
        remaining = &remaining[chunk..];
    }
    Ok(())
}

fn read_at(spool: &SpoolBacking, offset: u32, out: &mut [u8]) {
    let cap = spool.capacity;
    let mut phys = offset % cap;
    let mut written = 0usize;
    while written < out.len() {
        let until_wrap = (cap - phys) as usize;
        let chunk = (out.len() - written).min(until_wrap);
        let start = phys as usize;
        out[written..written + chunk].copy_from_slice(&spool.buf[start..start + chunk]);
        phys = (phys + chunk as u32) % cap;
        written += chunk;
    }
}

fn read_u32_at(spool: &SpoolBacking, offset: u32) -> Result<u32> {
    let mut bytes = [0u8; RECORD_LEN_BYTES as usize];
    read_at(spool, offset, &mut bytes);
    Ok(u32::from_le_bytes(bytes))
}

fn read_payload_logical(spool: &SpoolBacking, logical_off: u32, out: &mut [u8]) -> Result<usize> {
    if logical_off >= spool.meta.payload_bytes || out.is_empty() {
        return Ok(0);
    }

    let want = out
        .len()
        .min((spool.meta.payload_bytes - logical_off) as usize);
    let mut record = spool.meta.tail;
    let mut stored_remaining = spool.meta.stored_bytes;
    let mut payload_before = 0u32;
    let mut written = 0usize;

    while stored_remaining > 0 && written < want {
        if stored_remaining < RECORD_LEN_BYTES {
            return Err(anyhow!("spool truncated record prefix"));
        }
        let payload_len = read_u32_at(spool, record)?;
        let record_len = payload_len
            .checked_add(RECORD_LEN_BYTES)
            .ok_or_else(|| anyhow!("spool record length corrupt"))?;
        if record_len > stored_remaining {
            return Err(anyhow!("spool truncated record"));
        }

        let payload_end = payload_before.saturating_add(payload_len);
        if logical_off < payload_end {
            let within = logical_off.saturating_sub(payload_before);
            let available = (payload_len - within) as usize;
            let chunk = available.min(want - written);
            let payload_start = (record + RECORD_LEN_BYTES + within) % spool.capacity;
            read_at(spool, payload_start, &mut out[written..written + chunk]);
            written += chunk;
        }
        payload_before = payload_end;
        record = (record + record_len) % spool.capacity;
        stored_remaining -= record_len;
    }
    Ok(written)
}

#[must_use]
pub fn used_bytes() -> u32 {
    SPOOL
        .get()
        .and_then(|g| g.lock().ok())
        .map(|s| s.meta.payload_bytes)
        .unwrap_or(0)
}

/// Payload length and tail epoch for a replay snapshot.
#[must_use]
pub fn replay_snapshot() -> (u32, u32) {
    SPOOL
        .get()
        .and_then(|g| g.lock().ok())
        .map(|s| (s.meta.payload_bytes, s.meta.tail_epoch))
        .unwrap_or((0, 0))
}

#[must_use]
pub fn capacity_bytes() -> u32 {
    SPOOL
        .get()
        .and_then(|g| g.lock().ok())
        .map(|s| s.capacity)
        .unwrap_or(DEFAULT_SPOOL_CAPACITY)
}

#[must_use]
pub fn fill_percent() -> u8 {
    let cap = capacity_bytes();
    if cap == 0 {
        return 0;
    }
    let stored = SPOOL
        .get()
        .and_then(|g| g.lock().ok())
        .map(|s| s.meta.stored_bytes)
        .unwrap_or(0);
    ((stored as u64) * 100 / cap as u64).min(100) as u8
}

/// Read the opaque replay stream at a payload-only logical byte offset.
pub fn read_chunk(offset: u32, max_len: usize) -> Result<Vec<u8>> {
    with_spool_mut(|spool| {
        let mut buf = vec![0u8; max_len];
        let n = read_payload_logical(spool, offset, &mut buf)?;
        buf.truncate(n);
        Ok(buf)
    })
}

/// Clear spool after successful phone catch-up.
pub fn clear() -> Result<()> {
    with_spool_mut(|spool| {
        spool.meta = SpoolMeta {
            head: 0,
            tail: 0,
            stored_bytes: 0,
            payload_bytes: 0,
            tail_epoch: spool.meta.tail_epoch.wrapping_add(1),
        };
        Ok(())
    })
}

/// Remove exactly the complete records represented by a replay snapshot.
///
/// Returns `false` if overflow already advanced the tail; callers must replay the
/// current spool instead of clearing data from a changed snapshot.
pub fn clear_replayed_prefix(payload_bytes: u32, tail_epoch: u32) -> Result<bool> {
    with_spool_mut(|spool| {
        if spool.meta.tail_epoch != tail_epoch || payload_bytes > spool.meta.payload_bytes {
            return Ok(false);
        }
        let mut removed = 0u32;
        while removed < payload_bytes {
            let payload_len = read_u32_at(spool, spool.meta.tail)?;
            if removed.saturating_add(payload_len) > payload_bytes {
                return Err(anyhow!("replay snapshot ends inside spool record"));
            }
            drop_oldest_record(spool)?;
            removed = removed.saturating_add(payload_len);
        }
        Ok(removed == payload_bytes)
    })
}
