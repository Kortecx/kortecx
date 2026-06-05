//! [`JsonlAuditSink`] — a best-effort, line-delimited-JSON [`AuditSink`].
//!
//! One JSON object per line (JSONL): a SIEM / `jq -c` ingestion contract. Each
//! line carries a monotonic `seq`, a wall-clock `ts_ms`, an optional `principal`,
//! and the internally-tagged event body with ids rendered as lowercase hex
//! (see `crate::wire`). The open is fail-fast (surfaced at construction); every
//! record-time failure is swallowed + `warn`-logged + counted ([`Self::dropped`]),
//! so the run it audits can never fail because of audit I/O.
//!
//! Durability is deliberately *best-effort*: writes are buffered and there is NO
//! per-event `fsync` (that would serialize the run on disk latency). Buffered
//! lines are flushed on [`Self::flush`] (the orchestrator calls it at run-complete)
//! and again on `Drop` (the crash/early-return safety net). On a hard kill the
//! buffered tail may be lost — acceptable, because the journal is the durable
//! truth and the digest is recomputable from it.
//!
//! There is NO log rotation: the file grows append-only and the caller owns
//! retention/rotation (e.g. logrotate). A rotating backend is a future impl
//! behind the same [`AuditSink`] trait.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::AuditError;
use crate::event::AuditEvent;
use crate::sink::AuditSink;
use crate::wire::AuditEventWire;

/// A best-effort JSONL audit sink over a single append target.
#[derive(Debug)]
pub struct JsonlAuditSink {
    writer: Mutex<BufWriter<File>>,
    seq: AtomicU64,
    dropped: AtomicU64,
    principal: Option<String>,
}

impl JsonlAuditSink {
    /// Create (truncating any existing file) a fresh audit log at `path`. Use this
    /// for a single run (`kx run`), where each invocation starts a clean trail.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, AuditError> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Ok(Self::from_file(file))
    }

    /// Open `path` for append, creating it if absent (existing lines preserved).
    /// Use this for a long-lived process (`kx serve`) accumulating a trail across
    /// runs. Note: `seq` is per-sink and resets to 0 on each open — segment runs
    /// by the (future) `principal`/`run_id` envelope, not by a global `seq`.
    pub fn append(path: impl AsRef<Path>) -> Result<Self, AuditError> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self::from_file(file))
    }

    fn from_file(file: File) -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(file)),
            seq: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            principal: None,
        }
    }

    /// Stamp every line with a run-scoped `principal` ("who"). Absent by default;
    /// the gateway/auth layer supplies the authenticated principal (a follow-on).
    #[must_use]
    pub fn with_principal(mut self, principal: impl Into<String>) -> Self {
        self.principal = Some(principal.into());
        self
    }

    /// Serialize the line OUTSIDE the writer lock (small critical section), then
    /// append it. Returns the I/O result; the caller absorbs it.
    fn write_line(&self, event: &AuditEvent) -> std::io::Result<()> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let ts_ms = now_epoch_millis();
        let wire = AuditEventWire::from_event(seq, ts_ms, event, self.principal.clone());
        let mut line = serde_json::to_string(&wire).map_err(std::io::Error::other)?;
        line.push('\n');
        let mut w = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        w.write_all(line.as_bytes())
    }
}

impl AuditSink for JsonlAuditSink {
    fn record(&self, event: AuditEvent) {
        if let Err(e) = self.write_line(&event) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(error = %e, "audit: dropped event (jsonl write failed; best-effort)");
        }
    }

    fn flush(&self) {
        let mut w = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        if let Err(e) = w.flush() {
            tracing::warn!(error = %e, "audit: flush failed (best-effort)");
        }
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl Drop for JsonlAuditSink {
    fn drop(&mut self) {
        // Crash/early-return safety net: flush any buffered tail (RunCompleted +
        // the final commits) even if the orchestrator never called `flush()`.
        let mut w = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = w.flush();
    }
}

/// Wall-clock epoch milliseconds. OFF the digest/identity path — used only on the
/// JSONL wire. Saturates rather than panicking on a pre-epoch / overflowing clock.
fn now_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_mote::{MoteId, NdClass};
    use serde_json::Value;

    use super::*;
    use crate::event::DispatchKind;

    fn mid(b: u8) -> MoteId {
        MoteId::from_bytes([b; 32])
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(path).unwrap();
        text.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str::<Value>(l).expect("each line is valid JSON"))
            .collect()
    }

    #[test]
    fn writes_valid_jsonl_with_hex_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        {
            let sink = JsonlAuditSink::create(&path).unwrap();
            sink.record(AuditEvent::RunStarted { runnable: 8 });
            sink.record(AuditEvent::MoteDispatched {
                mote_id: mid(0xab),
                nd_class: NdClass::WorldMutating,
                kind: DispatchKind::WmFresh,
            });
            sink.record(AuditEvent::MoteCommitted {
                mote_id: mid(0xab),
                result_ref: ContentRef::from_bytes([0xcd; 32]),
                nd_class: NdClass::WorldMutating,
            });
            sink.record(AuditEvent::RunCompleted {
                committed: 8,
                total: 8,
                digest: [0xef; 32],
            });
            sink.flush();
        }
        let lines = read_lines(&path);
        assert_eq!(lines.len(), 4);

        // seq is gap-free from 0.
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(line["seq"], i as u64);
            assert!(line["ts_ms"].is_u64(), "ts_ms present and numeric");
        }
        // ids are 64-char lowercase hex, NOT an int array.
        assert_eq!(lines[0]["type"], "run_started");
        assert_eq!(lines[0]["runnable"], 8);
        assert_eq!(lines[1]["type"], "mote_dispatched");
        assert_eq!(lines[1]["mote_id"], "ab".repeat(32));
        assert_eq!(lines[1]["nd_class"], "world_mutating");
        assert_eq!(lines[1]["kind"], "wm_fresh");
        assert_eq!(lines[2]["result_ref"], "cd".repeat(32));
        assert_eq!(lines[3]["type"], "run_completed");
        assert_eq!(lines[3]["digest"], "ef".repeat(32));
        // No `principal` field unless set.
        assert!(lines[0].get("principal").is_none());
    }

    #[test]
    fn principal_is_stamped_when_set() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = JsonlAuditSink::create(&path)
            .unwrap()
            .with_principal("svc-account-7");
        sink.record(AuditEvent::RunStarted { runnable: 1 });
        sink.flush();
        let lines = read_lines(&path);
        assert_eq!(lines[0]["principal"], "svc-account-7");
    }

    #[test]
    fn open_failure_surfaces_at_construction() {
        // A path under a nonexistent directory cannot be created.
        let err = JsonlAuditSink::create("/no/such/dir/deeper/audit.jsonl");
        assert!(matches!(err, Err(AuditError::Open(_))));
    }

    #[test]
    fn flush_persists_buffered_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = JsonlAuditSink::create(&path).unwrap();
        sink.record(AuditEvent::RunStarted { runnable: 1 });
        sink.flush();
        assert_eq!(read_lines(&path).len(), 1);
    }

    #[test]
    fn drop_flushes_without_explicit_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        {
            let sink = JsonlAuditSink::create(&path).unwrap();
            sink.record(AuditEvent::RunStarted { runnable: 1 });
            // No explicit flush — Drop must persist it.
        }
        assert_eq!(read_lines(&path).len(), 1);
    }

    #[test]
    fn truncate_vs_append_semantics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        {
            let s = JsonlAuditSink::create(&path).unwrap();
            s.record(AuditEvent::RunStarted { runnable: 1 });
            s.flush();
        }
        // `create` truncates: a fresh run starts empty.
        {
            let s = JsonlAuditSink::create(&path).unwrap();
            s.record(AuditEvent::RunStarted { runnable: 2 });
            s.flush();
        }
        assert_eq!(read_lines(&path).len(), 1, "create truncates");
        // `append` preserves prior lines.
        {
            let s = JsonlAuditSink::append(&path).unwrap();
            s.record(AuditEvent::RunStarted { runnable: 3 });
            s.flush();
        }
        assert_eq!(read_lines(&path).len(), 2, "append preserves");
    }

    #[test]
    fn seq_is_unique_and_gapfree_under_concurrency() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let sink = Arc::new(JsonlAuditSink::create(&path).unwrap());
        let threads = 8;
        let per = 250;
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let s = Arc::clone(&sink);
                std::thread::spawn(move || {
                    for _ in 0..per {
                        s.record(AuditEvent::RunStarted { runnable: 1 });
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        sink.flush();
        let lines = read_lines(&path);
        assert_eq!(lines.len(), threads * per);
        let mut seqs: Vec<u64> = lines.iter().map(|l| l["seq"].as_u64().unwrap()).collect();
        seqs.sort_unstable();
        let expected: Vec<u64> = (0..(threads * per) as u64).collect();
        assert_eq!(seqs, expected, "seq set is exactly 0..N — unique, gap-free");
        assert_eq!(sink.dropped(), 0);
    }
}
