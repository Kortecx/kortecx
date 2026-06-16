//! Mote execution telemetry — the durable `telemetry.db` sidecar (Batch C).
//!
//! The HOST measures execution exhaust as motes actually run — wall-clock, the
//! model that ran (+ output tokens), the fired tool — and exposes it through
//! `ListMoteTelemetry` via the [`kx_gateway_core::TelemetryView`] seam.
//!
//! ## Off the truth path, rebuildable to EMPTY (the load-bearing boundary)
//! Telemetry is never journaled, never a `MoteId` input, never gating
//! execution, never a digest input. UNLIKE capture (journal-derived,
//! refoldable) the exec metrics are NOT journal-derivable — a stale/corrupt
//! sidecar rebuilds to EMPTY (the uploads.db posture): dropping it loses
//! observability, not truth.
//!
//! ## The hot path can NEVER block, slow, or fail a run (fail-open)
//! [`TelemetryExecutor`] wraps the worker's executor chain and records through
//! a **bounded** channel with `try_send` — a full queue or a stalled disk DROPS
//! the event (debug-logged) and the wrapper returns the inner result verbatim
//! on every path. The cost on the mote loop is one `Instant` read + one
//! `try_send`.
//!
//! ## The join (order-robust, the capture.rs posture)
//! Exec/usage events carry only `mote_id`; the background tick
//! ([`TelemetryLedger::join_fold`]) drains the channel AND folds the journal
//! forward, joining rows to their `Committed` fact's `seq` + the
//! watermark-attributed `instance_id` (the latest `RunRegistered` at-or-below —
//! the capture.db `run_meta` precedent) through a durable `commits` join table,
//! so either arrival order (event-then-fact or fact-then-event) lands the same
//! row. `ListMoteTelemetry` surfaces only joined rows (`seq IS NOT NULL`) — an
//! executed-but-never-committed mote (dead-letter) never lists.

use std::path::Path;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use kx_executor::{MoteExecutor, MoteExecutorError, Rootfs};
use kx_gateway_core::{
    JournalReader, ModelTokenRollup, MoteTelemetryEntry, TelemetrySummary, TelemetryView,
};
use kx_journal::JournalEntry;
use kx_mote::Mote;
use kx_warrant::ExecutorClass;
use rusqlite::{params, Connection};

use crate::error::GatewayError;
use kx_gateway_core::GatewayError as CoreError;

/// The telemetry sidecar schema version. A bump (or any decode failure) makes
/// the load path drop and rebuild to EMPTY — exec metrics are not
/// journal-derivable, so there is never a migration OR a refold, only a fresh
/// start (the honest semantics for exhaust).
const SCHEMA_VERSION: i64 = 1;

/// Bounded event-queue depth between the worker hot path and the background
/// writer tick. At the 250 ms tick cadence this absorbs ~4k events/s before
/// dropping — far beyond the single-writer commit ceiling. Full ⇒ drop.
const EVENT_QUEUE: usize = 1024;

/// The durable schema (idempotent). `exec_metrics` is the exhaust table
/// (`input_tokens` is reserved — NEVER written in OSS: the frozen backend seam
/// reports no input count); `commits` is the order-robust journal join source;
/// `meta` holds the fold watermark + schema version; `run_meta` the durable
/// run-attribution watermark (the capture.db precedent).
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS exec_metrics (
    mote_id         BLOB PRIMARY KEY,
    started_unix_ms INTEGER NOT NULL DEFAULT 0,
    wall_clock_ms   INTEGER NOT NULL DEFAULT 0,
    input_tokens    INTEGER,
    output_tokens   INTEGER,
    model_id        TEXT NOT NULL DEFAULT '',
    tool_id         TEXT NOT NULL DEFAULT '',
    instance_id     BLOB NOT NULL DEFAULT x'',
    seq             INTEGER
);
CREATE INDEX IF NOT EXISTS telemetry_by_seq ON exec_metrics(seq DESC);
CREATE TABLE IF NOT EXISTS commits (
    mote_id     BLOB PRIMARY KEY,
    seq         INTEGER NOT NULL,
    instance_id BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS run_meta (id INTEGER PRIMARY KEY CHECK (id = 0), instance_id BLOB NOT NULL);";

/// One measurement from the execution hot path. Carries only what the wrapper
/// can see at run time; the journal join (seq + instance) happens in the tick.
#[derive(Debug)]
enum TelemetryEvent {
    /// The executor wrapper's measurement of one successful `run`.
    Exec {
        mote_id: [u8; 32],
        started_unix_ms: u64,
        wall_clock_ms: u64,
        tool_id: String,
    },
    /// A model dispatch's usage (the inference build's `UsageSink` hook).
    /// Constructed only by `record_usage` — the FFI-free build has no model
    /// dispatch, so the variant is dead there by design.
    #[cfg_attr(not(feature = "inference"), allow(dead_code))]
    Usage {
        mote_id: [u8; 32],
        model_id: String,
        output_tokens: u64,
    },
}

/// The model-usage hook the inference build's `ModelRouterExecutor` records
/// through (kept trait-shaped so `model_exec` needs no telemetry type beyond
/// one `Arc<dyn UsageSink>`). Implementations MUST be non-blocking + infallible
/// from the caller's view (the fail-open posture). Dead on the FFI-free build
/// (no model dispatch exists to record).
#[cfg_attr(not(feature = "inference"), allow(dead_code))]
pub(crate) trait UsageSink: Send + Sync {
    /// Record that a model dispatch for `mote_id` actually ran `model_id` and
    /// emitted `output_tokens`. Never blocks; never fails the caller.
    fn record_usage(&self, mote_id: [u8; 32], model_id: &str, output_tokens: u64);
}

/// The cloneable hot-path handle: a bounded `try_send` into the ledger's event
/// queue. Full/disconnected ⇒ the event is dropped (debug-logged) — telemetry
/// loses a row, the run is untouched.
#[derive(Clone)]
pub(crate) struct TelemetrySink {
    tx: SyncSender<TelemetryEvent>,
}

impl TelemetrySink {
    fn send(&self, event: TelemetryEvent) {
        match self.tx.try_send(event) {
            // Disconnected = the ledger is gone (shutdown) — the same silent
            // drop as success-with-nobody-listening.
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(event)) => {
                tracing::debug!(?event, "telemetry queue full — event dropped (fail-open)");
            }
        }
    }
}

impl UsageSink for TelemetrySink {
    fn record_usage(&self, mote_id: [u8; 32], model_id: &str, output_tokens: u64) {
        self.send(TelemetryEvent::Usage {
            mote_id,
            model_id: model_id.to_string(),
            output_tokens,
        });
    }
}

/// The durable telemetry ledger over `telemetry.db`. A single mutex'd
/// connection serves the periodic join tick (write) and the
/// `ListMoteTelemetry` read — telemetry is low-traffic (one page read; a
/// periodic batched drain), so the mutex is never contended.
pub(crate) struct TelemetryLedger {
    conn: Mutex<Connection>,
    /// The drained side of the bounded event queue (the tick owns it).
    rx: Mutex<Receiver<TelemetryEvent>>,
    /// The template sender [`TelemetryLedger::sink`] clones from.
    tx: SyncSender<TelemetryEvent>,
}

impl TelemetryLedger {
    /// Open (or create) `telemetry.db` under `dir`. A `schema_version` mismatch
    /// or a corrupt/foreign sidecar drops every table and starts EMPTY — exec
    /// metrics are not journal-derivable, so a rebuild is a fresh start, never
    /// a refold. Idempotent on restart.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("telemetry dir: {e}")))?;
        let db_path = dir.join("telemetry.db");
        // A non-SQLite file fails even the pragma; delete + recreate (the
        // capture.rs corrupt-file posture — this sidecar holds no truth).
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("telemetry.db-wal"));
            let _ = std::fs::remove_file(dir.join("telemetry.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("telemetry reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS exec_metrics;
                 DROP TABLE IF EXISTS commits;
                 DROP TABLE IF EXISTS run_meta;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("telemetry rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("telemetry schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1), ('last_seq', 0)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("telemetry meta init: {e}")))?;
        let (tx, rx) = std::sync::mpsc::sync_channel(EVENT_QUEUE);
        Ok(Self {
            conn: Mutex::new(conn),
            rx: Mutex::new(rx),
            tx,
        })
    }

    fn open_with_pragma(db_path: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;",
        )?;
        Ok(conn)
    }

    fn read_schema_version(conn: &Connection) -> rusqlite::Result<Option<i64>> {
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }

    /// A new hot-path handle into this ledger's bounded event queue.
    pub(crate) fn sink(&self) -> TelemetrySink {
        TelemetrySink {
            tx: self.tx.clone(),
        }
    }

    /// One background tick: drain the queued exec/usage events into
    /// `exec_metrics` (order-robust upsert), fold the journal forward from the
    /// durable watermark (stamping `seq` + watermark `instance_id` through the
    /// `commits` join table), and back-join any rows either side completed.
    /// One transaction per tick, watermark last; a fault is logged and the next
    /// tick retries (fail-safe, never panics on the background task). Returns
    /// the number of rows newly joined (listable) this tick.
    // A flat drain-fold-join over the event queue + journal range — the length
    // is the arm count, not cognitive complexity (the capture::fold precedent).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn join_fold(&self, reader: &dyn JournalReader) -> usize {
        let head = match reader.current_seq() {
            Ok(h) => h,
            Err(error) => {
                tracing::warn!(%error, "telemetry tick: journal head read failed");
                return 0;
            }
        };
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let from = u64::try_from(Self::watermark(&conn)).unwrap_or(0);
        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(error) => {
                tracing::warn!(%error, "telemetry tick: begin txn failed");
                return 0;
            }
        };

        // (1) Drain the hot-path events (bounded queue ⇒ bounded batch).
        if let Ok(rx) = self.rx.lock() {
            for event in rx.try_iter() {
                match event {
                    TelemetryEvent::Exec {
                        mote_id,
                        started_unix_ms,
                        wall_clock_ms,
                        tool_id,
                    } => {
                        let _ = tx.execute(
                            "INSERT INTO exec_metrics(mote_id, started_unix_ms, wall_clock_ms, tool_id) \
                             VALUES (?1, ?2, ?3, ?4) \
                             ON CONFLICT(mote_id) DO UPDATE SET \
                               started_unix_ms = excluded.started_unix_ms, \
                               wall_clock_ms = excluded.wall_clock_ms, \
                               tool_id = excluded.tool_id",
                            params![
                                mote_id.to_vec(),
                                i64::try_from(started_unix_ms).unwrap_or(i64::MAX),
                                i64::try_from(wall_clock_ms).unwrap_or(i64::MAX),
                                tool_id,
                            ],
                        );
                    }
                    TelemetryEvent::Usage {
                        mote_id,
                        model_id,
                        output_tokens,
                    } => {
                        let _ = tx.execute(
                            "INSERT INTO exec_metrics(mote_id, model_id, output_tokens) \
                             VALUES (?1, ?2, ?3) \
                             ON CONFLICT(mote_id) DO UPDATE SET \
                               model_id = excluded.model_id, \
                               output_tokens = excluded.output_tokens",
                            params![
                                mote_id.to_vec(),
                                model_id,
                                i64::try_from(output_tokens).unwrap_or(i64::MAX),
                            ],
                        );
                    }
                }
            }
        }

        // (2) Fold the journal forward: every Committed fact lands in the
        // durable `commits` join table with the watermark attribution in force
        // at its seq (entries arrive in ascending seq, so the in-pass watermark
        // is exact even across interleaved runs).
        if head > from {
            let mut instance: Vec<u8> = Self::run_instance(&tx).unwrap_or_default();
            match reader.read_entries_by_seq(from + 1..head.saturating_add(1)) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            JournalEntry::RunRegistered { instance_id, .. } => {
                                instance = instance_id.to_vec();
                                let _ = tx.execute(
                                    "INSERT OR REPLACE INTO run_meta(id, instance_id) VALUES (0, ?1)",
                                    params![instance.clone()],
                                );
                            }
                            JournalEntry::Committed { mote_id, seq, .. } => {
                                let _ = tx.execute(
                                    "INSERT OR REPLACE INTO commits(mote_id, seq, instance_id) \
                                     VALUES (?1, ?2, ?3)",
                                    params![
                                        mote_id.as_bytes().to_vec(),
                                        i64::try_from(seq).unwrap_or(i64::MAX),
                                        instance.clone(),
                                    ],
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "telemetry tick: range read failed");
                }
            }
            let _ = tx.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('last_seq', ?1)",
                params![i64::try_from(head).unwrap_or(i64::MAX)],
            );
        }

        // (3) The order-robust back-join: stamp every still-unjoined exec row
        // whose commit fact is known (covers event-then-fact AND fact-then-event
        // arrival, across any tick split).
        let joined = tx
            .execute(
                "UPDATE exec_metrics SET \
                   seq = (SELECT c.seq FROM commits c WHERE c.mote_id = exec_metrics.mote_id), \
                   instance_id = (SELECT c.instance_id FROM commits c WHERE c.mote_id = exec_metrics.mote_id) \
                 WHERE seq IS NULL \
                   AND mote_id IN (SELECT mote_id FROM commits)",
                [],
            )
            .unwrap_or(0);

        if let Err(error) = tx.commit() {
            tracing::warn!(%error, "telemetry tick: commit failed (retried next tick)");
            return 0;
        }
        joined
    }

    fn watermark(conn: &Connection) -> i64 {
        conn.query_row("SELECT value FROM meta WHERE key = 'last_seq'", [], |r| {
            r.get(0)
        })
        .unwrap_or(0)
    }

    /// The durable run-attribution watermark (the latest folded `RunRegistered`).
    fn run_instance(tx: &rusqlite::Transaction<'_>) -> Option<Vec<u8>> {
        tx.query_row("SELECT instance_id FROM run_meta WHERE id = 0", [], |r| {
            r.get(0)
        })
        .ok()
    }
}

impl TelemetryView for TelemetryLedger {
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        mote_id: Option<[u8; 32]>,
        before_seq: Option<u64>,
    ) -> Result<(Vec<MoteTelemetryEntry>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("telemetry lock poisoned".into()))?;
        // Composable filters over one base query; only JOINED rows surface
        // (`seq IS NOT NULL` — an uncommitted execution never lists).
        let mut sql = String::from(
            "SELECT mote_id, instance_id, wall_clock_ms, input_tokens, output_tokens, \
                    model_id, tool_id, started_unix_ms, seq \
             FROM exec_metrics WHERE seq IS NOT NULL",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(inst) = instance_id {
            sql.push_str(" AND instance_id = ?");
            args.push(Box::new(inst.to_vec()));
        }
        if let Some(mote) = mote_id {
            sql.push_str(" AND mote_id = ?");
            args.push(Box::new(mote.to_vec()));
        }
        if let Some(before) = before_seq {
            sql.push_str(" AND seq < ?");
            args.push(Box::new(i64::try_from(before).unwrap_or(i64::MAX)));
        }
        sql.push_str(" ORDER BY seq DESC LIMIT ?");
        // Over-fetch by one to compute `has_more` without a second COUNT query.
        let over = limit.saturating_add(1);
        args.push(Box::new(i64::try_from(over).unwrap_or(i64::MAX)));

        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CoreError::Internal(format!("telemetry query prep: {e}")))?;
        let mapped = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter().map(std::convert::AsRef::as_ref)),
                |r| {
                    let mote: Vec<u8> = r.get(0)?;
                    let inst: Vec<u8> = r.get(1)?;
                    let wall: i64 = r.get(2)?;
                    let input_tokens: Option<i64> = r.get(3)?;
                    let output_tokens: Option<i64> = r.get(4)?;
                    let model_id: String = r.get(5)?;
                    let tool_id: String = r.get(6)?;
                    let started: i64 = r.get(7)?;
                    let seq: i64 = r.get(8)?;
                    Ok(MoteTelemetryEntry {
                        mote_id: <[u8; 32]>::try_from(mote.as_slice()).unwrap_or([0; 32]),
                        instance_id: <[u8; 16]>::try_from(inst.as_slice()).unwrap_or([0; 16]),
                        wall_clock_ms: u64::try_from(wall).unwrap_or(0),
                        input_tokens: input_tokens.map(|t| u64::try_from(t).unwrap_or(0)),
                        output_tokens: output_tokens.map(|t| u64::try_from(t).unwrap_or(0)),
                        model_id,
                        tool_id,
                        started_unix_ms: u64::try_from(started).unwrap_or(0),
                        seq: u64::try_from(seq).unwrap_or(0),
                    })
                },
            )
            .map_err(|e| CoreError::Internal(format!("telemetry query: {e}")))?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(|e| CoreError::Internal(format!("telemetry row: {e}")))?);
        }
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok((rows, has_more))
    }

    fn summarize(&self, instance_id: Option<[u8; 16]>) -> Result<TelemetrySummary, CoreError> {
        // The EXACT, cross-page rollup: a single GROUP BY over the whole scope
        // (never a page window), so a long ReAct run is summed honestly. Only
        // JOINED rows count (`seq IS NOT NULL` — an uncommitted execution never
        // contributes). Per-model rows exclude non-model motes (empty model_id)
        // but they still count toward `total_motes` (computed separately).
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("telemetry lock poisoned".into()))?;

        // (1) Per-model rollups — empty model_id excluded from these rows.
        let mut sql = String::from(
            "SELECT model_id, COUNT(*), \
                    COALESCE(SUM(output_tokens), 0), COALESCE(SUM(wall_clock_ms), 0) \
             FROM exec_metrics WHERE seq IS NOT NULL AND model_id <> ''",
        );
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(inst) = instance_id {
            sql.push_str(" AND instance_id = ?");
            args.push(Box::new(inst.to_vec()));
        }
        sql.push_str(" GROUP BY model_id");
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| CoreError::Internal(format!("telemetry summary prep: {e}")))?;
        let mapped = stmt
            .query_map(
                rusqlite::params_from_iter(args.iter().map(std::convert::AsRef::as_ref)),
                |r| {
                    let model_id: String = r.get(0)?;
                    let count: i64 = r.get(1)?;
                    let out: i64 = r.get(2)?;
                    let wall: i64 = r.get(3)?;
                    Ok(ModelTokenRollup {
                        model_id,
                        count: u64::try_from(count).unwrap_or(0),
                        total_output_tokens: u64::try_from(out).unwrap_or(0),
                        total_wall_clock_ms: u64::try_from(wall).unwrap_or(0),
                    })
                },
            )
            .map_err(|e| CoreError::Internal(format!("telemetry summary query: {e}")))?;
        let mut rows = Vec::new();
        for row in mapped {
            rows.push(row.map_err(|e| CoreError::Internal(format!("telemetry summary row: {e}")))?);
        }
        // Descending output tokens; ties by model_id (mirrors the seam default).
        rows.sort_by(|a: &ModelTokenRollup, b: &ModelTokenRollup| {
            b.total_output_tokens
                .cmp(&a.total_output_tokens)
                .then_with(|| a.model_id.cmp(&b.model_id))
        });

        // (2) Window-wide totals over ALL joined motes (model + non-model).
        let mut tsql = String::from(
            "SELECT COUNT(*), COALESCE(SUM(output_tokens), 0) \
             FROM exec_metrics WHERE seq IS NOT NULL",
        );
        let mut targs: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(inst) = instance_id {
            tsql.push_str(" AND instance_id = ?");
            targs.push(Box::new(inst.to_vec()));
        }
        let (total_motes, total_output_tokens): (u64, u64) = conn
            .query_row(
                &tsql,
                rusqlite::params_from_iter(targs.iter().map(std::convert::AsRef::as_ref)),
                |r| {
                    let motes: i64 = r.get(0)?;
                    let out: i64 = r.get(1)?;
                    Ok((
                        u64::try_from(motes).unwrap_or(0),
                        u64::try_from(out).unwrap_or(0),
                    ))
                },
            )
            .map_err(|e| CoreError::Internal(format!("telemetry summary totals: {e}")))?;

        Ok(TelemetrySummary {
            rows,
            total_motes,
            total_output_tokens,
        })
    }
}

/// The OUTERMOST executor wrapper: measures every leased mote's wall clock and
/// records it through the bounded sink. Structurally fail-open — it returns
/// `inner`'s result verbatim on every path (success AND error), and the only
/// added work on the hot loop is two clock reads + one `try_send`.
pub(crate) struct TelemetryExecutor {
    inner: std::sync::Arc<dyn MoteExecutor>,
    sink: TelemetrySink,
}

impl TelemetryExecutor {
    pub(crate) fn new(inner: std::sync::Arc<dyn MoteExecutor>, sink: TelemetrySink) -> Self {
        Self { inner, sink }
    }

    /// The display tool id of a tool-bearing mote (`name@version`, the
    /// `mcp-echo@1` convention); empty when the contract is empty.
    fn tool_id_of(mote: &Mote) -> String {
        mote.def
            .tool_contract
            .iter()
            .next()
            .map(|(name, version)| format!("{}@{}", name.0, version.0))
            .unwrap_or_default()
    }
}

impl MoteExecutor for TelemetryExecutor {
    fn run(
        &self,
        mote: &Mote,
        warrant: &kx_warrant::WarrantSpec,
        env: Option<Rootfs>,
    ) -> Result<kx_executor::MoteExecutionResult, MoteExecutorError> {
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        let started = Instant::now();
        let result = self.inner.run(mote, warrant, env);
        if result.is_ok() {
            self.sink.send(TelemetryEvent::Exec {
                mote_id: *mote.id.as_bytes(),
                started_unix_ms,
                wall_clock_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                tool_id: Self::tool_id_of(mote),
            });
        }
        result
    }

    fn supports(&self, executor_class: ExecutorClass) -> bool {
        self.inner.supports(executor_class)
    }
}

#[cfg(test)]
mod tests {
    use kx_content::ContentRef;
    use kx_journal::{InMemoryJournal, Journal, INSTANCE_ID_LEN};
    use kx_mote::{MoteDefHash, MoteId, NdClass};
    use smallvec::SmallVec;

    use kx_gateway_core::ReadOnly;

    use super::*;

    fn committed(mote: u8) -> JournalEntry {
        JournalEntry::Committed {
            mote_id: MoteId::from_bytes([mote; 32]),
            idempotency_key: [mote; 32],
            seq: 0, // journal assigns on append
            nondeterminism: NdClass::Pure,
            result_ref: ContentRef::from_bytes([mote; 32]),
            parents: SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
        }
    }

    fn registered(instance: u8) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [instance; INSTANCE_ID_LEN],
            recipe_fingerprint: [instance; 32],
            ts: 0,
            seq: 0,
        }
    }

    /// A journal that GROWS between ticks (the capture.rs test fixture — the
    /// read-only seam deliberately exposes no write surface).
    struct GrowableReader {
        entries: std::sync::RwLock<Vec<JournalEntry>>,
    }

    impl GrowableReader {
        fn new() -> Self {
            Self {
                entries: std::sync::RwLock::new(Vec::new()),
            }
        }

        fn push(&self, mut entry: JournalEntry, seq: u64) {
            match &mut entry {
                JournalEntry::Committed { seq: s, .. }
                | JournalEntry::RunRegistered { seq: s, .. } => *s = seq,
                _ => {}
            }
            self.entries.write().unwrap().push(entry);
        }
    }

    impl JournalReader for GrowableReader {
        fn read_entries_by_seq(
            &self,
            range: std::ops::Range<u64>,
        ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, kx_journal::JournalError> {
            let hit: Vec<JournalEntry> = self
                .entries
                .read()
                .unwrap()
                .iter()
                .filter(|e| range.contains(&e.seq()))
                .cloned()
                .collect();
            Ok(Box::new(hit.into_iter()))
        }

        fn current_seq(&self) -> Result<u64, kx_journal::JournalError> {
            Ok(self
                .entries
                .read()
                .unwrap()
                .iter()
                .map(JournalEntry::seq)
                .max()
                .unwrap_or(0))
        }
    }

    fn exec_event(mote: u8, wall_ms: u64) -> TelemetryEvent {
        TelemetryEvent::Exec {
            mote_id: [mote; 32],
            started_unix_ms: 1_000,
            wall_clock_ms: wall_ms,
            tool_id: String::new(),
        }
    }

    #[test]
    fn exec_then_usage_merge_is_order_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        // Mote 0x10: exec first, usage second. Mote 0x11: usage first, exec second.
        sink.send(exec_event(0x10, 12));
        sink.record_usage([0x10; 32], "kx-serve:qwen3", 42);
        sink.record_usage([0x11; 32], "kx-serve:qwen3", 7);
        sink.send(exec_event(0x11, 34));

        let j = InMemoryJournal::new();
        j.append(registered(5)).unwrap();
        j.append(committed(0x10)).unwrap();
        j.append(committed(0x11)).unwrap();
        let reader = ReadOnly::new(j);
        assert_eq!(ledger.join_fold(&reader), 2, "both rows joined this tick");

        let (rows, has_more) = ledger.list(10, None, None, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(!has_more);
        for row in &rows {
            assert_eq!(row.model_id, "kx-serve:qwen3", "usage merged either order");
            assert!(row.wall_clock_ms > 0, "exec merged either order");
            assert_eq!(row.instance_id, [5; INSTANCE_ID_LEN]);
            assert!(row.input_tokens.is_none(), "never set in OSS");
        }
        // Newest-first by seq.
        assert!(rows[0].seq > rows[1].seq);
        assert_eq!(rows[0].output_tokens, Some(7));
        assert_eq!(rows[1].output_tokens, Some(42));
    }

    #[test]
    fn join_is_order_robust_across_ticks() {
        // Fact folds in tick 1, the exec event drains in tick 2 (the worker's
        // channel can lag the journal) — the back-join must still stamp it.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let reader = GrowableReader::new();
        reader.push(registered(7), 1);
        reader.push(committed(0x20), 2);
        assert_eq!(
            ledger.join_fold(&reader),
            0,
            "no exec row yet — nothing joins"
        );

        // Tick 2: the exec event arrives AFTER its fact already folded.
        ledger.sink().send(exec_event(0x20, 5));
        assert_eq!(ledger.join_fold(&reader), 1, "back-joined from commits");

        let (rows, _) = ledger.list(10, None, None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seq, 2);
        assert_eq!(rows[0].instance_id, [7; INSTANCE_ID_LEN]);

        // And the reverse split: event in tick 3, fact in tick 4.
        ledger.sink().send(exec_event(0x21, 6));
        assert_eq!(ledger.join_fold(&reader), 0, "no fact yet — unjoined");
        let (rows, _) = ledger.list(10, None, None, None).unwrap();
        assert_eq!(rows.len(), 1, "an uncommitted execution never lists");
        reader.push(committed(0x21), 3);
        assert_eq!(ledger.join_fold(&reader), 1);
        let (rows, _) = ledger.list(10, None, None, None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn watermark_attribution_survives_ticks_and_runs() {
        // Two runs across ticks: commits attribute to the LATEST registration
        // at fold time (the capture run_meta precedent).
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let reader = GrowableReader::new();
        reader.push(registered(0xa1), 1);
        ledger.join_fold(&reader); // registration folds alone (durable watermark)

        ledger.sink().send(exec_event(0x30, 1));
        reader.push(committed(0x30), 2);
        ledger.join_fold(&reader);

        reader.push(registered(0xb2), 3);
        ledger.sink().send(exec_event(0x31, 2));
        reader.push(committed(0x31), 4);
        ledger.join_fold(&reader);

        let (rows, _) = ledger.list(10, None, None, None).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].instance_id, [0xb2; INSTANCE_ID_LEN], "run B");
        assert_eq!(rows[1].instance_id, [0xa1; INSTANCE_ID_LEN], "run A");
        // The instance filter scopes.
        let (only_b, _) = ledger
            .list(10, Some([0xb2; INSTANCE_ID_LEN]), None, None)
            .unwrap();
        assert_eq!(only_b.len(), 1);
        assert_eq!(only_b[0].mote_id, [0x31; 32]);
    }

    #[test]
    fn full_channel_drops_without_blocking() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        // Overfill the bounded queue (cap EVENT_QUEUE); every call must return
        // promptly — drop-on-full is the fail-open contract.
        for i in 0..(EVENT_QUEUE + 200) {
            #[allow(clippy::cast_possible_truncation)]
            sink.send(exec_event((i % 256) as u8, 1));
        }
        // Nothing to assert beyond "we got here without blocking": the drain
        // below proves at most EVENT_QUEUE events survived.
        let j = InMemoryJournal::new();
        j.append(registered(1)).unwrap();
        let reader = ReadOnly::new(j);
        ledger.join_fold(&reader);
        let conn = ledger.conn.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM exec_metrics", [], |r| r.get(0))
            .unwrap();
        assert!(
            count <= i64::try_from(EVENT_QUEUE).unwrap(),
            "the queue bound held (overflow dropped)"
        );
    }

    #[test]
    fn a_schema_version_bump_rebuilds_to_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let ledger = TelemetryLedger::open(dir.path()).unwrap();
            ledger.sink().send(exec_event(0x40, 9));
            let j = InMemoryJournal::new();
            j.append(registered(1)).unwrap();
            j.append(committed(0x40)).unwrap();
            assert_eq!(ledger.join_fold(&ReadOnly::new(j)), 1);
        }
        {
            let conn = Connection::open(dir.path().join("telemetry.db")).unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', 999)",
                [],
            )
            .unwrap();
        }
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let (rows, _) = ledger.list(10, None, None, None).unwrap();
        assert!(
            rows.is_empty(),
            "exec metrics are not journal-derivable: a stale schema rebuilds to EMPTY"
        );
    }

    /// GR10 M8b — the per-event hot-path cost of the telemetry sink (two clock
    /// reads happen in the executor wrapper; this measures the `try_send` +
    /// construction, including the drop-on-full path once the queue caps —
    /// exactly the overloaded-worker shape). Run in release via `just
    /// scale-smoke`'s suite or directly with `--ignored`.
    #[test]
    #[ignore = "GR10 measurement — meaningful in release only"]
    fn m8b_sink_per_event_cost() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        let n = 100_000u32;
        let t = std::time::Instant::now();
        for i in 0..n {
            #[allow(clippy::cast_possible_truncation)]
            sink.send(exec_event((i % 256) as u8, 1));
        }
        let per_event = t.elapsed() / n;
        println!("GR10 M8b telemetry sink per-event {per_event:?}");
        assert!(
            per_event < std::time::Duration::from_micros(5),
            "the hot-path sink stays sub-5µs per event"
        );
    }

    #[test]
    fn pagination_envelope_walks_1500_rows_without_dup_or_miss() {
        // The GR12 scale envelope for the read path: ≥1k rows paged by
        // before_seq cursors stay strictly descending, no dup, no miss.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let reader = GrowableReader::new();
        reader.push(registered(1), 1);
        let sink = ledger.sink();
        let total = 1_500u32;
        for i in 0..total {
            let mut id = [0u8; 32];
            id[..4].copy_from_slice(&i.to_le_bytes());
            sink.send(TelemetryEvent::Exec {
                mote_id: id,
                started_unix_ms: 1,
                wall_clock_ms: 1,
                tool_id: String::new(),
            });
            reader.push(
                JournalEntry::Committed {
                    mote_id: MoteId::from_bytes(id),
                    idempotency_key: id,
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    result_ref: ContentRef::from_bytes(id),
                    parents: SmallVec::new(),
                    warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                    mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
                },
                u64::from(i) + 2,
            );
            // The bounded queue (EVENT_QUEUE) caps a single burst — drain as we
            // go, exactly as the production 250 ms tick interleaves.
            if i % 500 == 499 {
                ledger.join_fold(&reader);
            }
        }
        ledger.join_fold(&reader);

        let mut walked: Vec<u64> = Vec::new();
        let mut cursor: Option<u64> = None;
        loop {
            let (page, has_more) = ledger.list(200, None, None, cursor).unwrap();
            let Some(last) = page.last() else { break };
            cursor = Some(last.seq);
            walked.extend(page.iter().map(|r| r.seq));
            if !has_more {
                break;
            }
        }
        assert_eq!(walked.len(), total as usize, "every row paged exactly once");
        assert!(
            walked.windows(2).all(|w| w[0] > w[1]),
            "strictly descending — no dup, no miss"
        );
    }

    #[test]
    fn before_seq_paginates_newest_first_to_exhaustion() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        let j = InMemoryJournal::new();
        j.append(registered(1)).unwrap();
        for i in 0u8..5 {
            sink.send(exec_event(0x50 + i, u64::from(i) + 1));
            j.append(committed(0x50 + i)).unwrap();
        }
        let reader = ReadOnly::new(j);
        assert_eq!(ledger.join_fold(&reader), 5);

        let (p1, more1) = ledger.list(2, None, None, None).unwrap();
        assert_eq!(p1.len(), 2);
        assert!(more1);
        let (p2, more2) = ledger.list(2, None, None, Some(p1[1].seq)).unwrap();
        assert_eq!(p2.len(), 2);
        assert!(more2);
        let (p3, more3) = ledger.list(2, None, None, Some(p2[1].seq)).unwrap();
        assert_eq!(p3.len(), 1);
        assert!(!more3);
        // No dup/miss across the walk; strictly descending seq.
        let walked: Vec<u64> = p1.iter().chain(&p2).chain(&p3).map(|r| r.seq).collect();
        let mut sorted = walked.clone();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        sorted.dedup();
        assert_eq!(walked, sorted);
        // The mote filter scopes to one row.
        let (one, _) = ledger.list(10, None, Some([0x52; 32]), None).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].mote_id, [0x52; 32]);
    }

    #[test]
    fn summary_groups_by_model_sums_tokens_and_wall() {
        // W1a-3: the exact GROUP BY rollup. Three motes on model A, one on B,
        // one non-model (echo) mote — per-model rows exclude the echo, but it
        // still counts toward total_motes.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        let j = InMemoryJournal::new();
        j.append(registered(1)).unwrap();
        // model A: motes 0x10..0x13 (3) — usage 10/20/30, wall 1/2/3
        for (i, (tok, wall)) in [(10u64, 1u64), (20, 2), (30, 3)].into_iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let m = 0x10u8 + i as u8;
            sink.send(exec_event(m, wall));
            sink.record_usage([m; 32], "model-a", tok);
            j.append(committed(m)).unwrap();
        }
        // model B: mote 0x20 — usage 5, wall 7
        sink.send(exec_event(0x20, 7));
        sink.record_usage([0x20; 32], "model-b", 5);
        j.append(committed(0x20)).unwrap();
        // non-model echo mote 0x30 — exec only (empty model_id), wall 4
        sink.send(exec_event(0x30, 4));
        j.append(committed(0x30)).unwrap();

        assert_eq!(ledger.join_fold(&ReadOnly::new(j)), 5);

        let s = ledger.summarize(None).unwrap();
        // Per-model rows: A then B (descending output tokens).
        assert_eq!(s.rows.len(), 2, "echo mote excluded from per-model rows");
        assert_eq!(s.rows[0].model_id, "model-a");
        assert_eq!(s.rows[0].count, 3);
        assert_eq!(s.rows[0].total_output_tokens, 60);
        assert_eq!(s.rows[0].total_wall_clock_ms, 6);
        assert_eq!(s.rows[1].model_id, "model-b");
        assert_eq!(s.rows[1].total_output_tokens, 5);
        // Window-wide totals count the echo mote too.
        assert_eq!(s.total_motes, 5, "echo counted in total_motes");
        assert_eq!(s.total_output_tokens, 65);
    }

    #[test]
    fn summary_scopes_to_instance() {
        // Two runs, distinct instance watermarks; the filter isolates one.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        let reader = GrowableReader::new();
        // run A (instance 0x07): mote 0x10, usage 11
        reader.push(registered(0x07), 1);
        sink.send(exec_event(0x10, 1));
        sink.record_usage([0x10; 32], "model-a", 11);
        reader.push(committed(0x10), 2);
        // run B (instance 0x08): mote 0x11, usage 22
        reader.push(registered(0x08), 3);
        sink.send(exec_event(0x11, 1));
        sink.record_usage([0x11; 32], "model-a", 22);
        reader.push(committed(0x11), 4);
        ledger.join_fold(&reader);

        let all = ledger.summarize(None).unwrap();
        assert_eq!(all.total_output_tokens, 33, "both runs summed");
        let a = ledger.summarize(Some([0x07; INSTANCE_ID_LEN])).unwrap();
        assert_eq!(a.total_motes, 1);
        assert_eq!(a.total_output_tokens, 11, "scoped to run A only");
        assert_eq!(a.rows[0].count, 1);
    }

    #[test]
    fn summary_excludes_unjoined_executions() {
        // An executed-but-never-committed mote (seq IS NULL) never contributes.
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let sink = ledger.sink();
        let j = InMemoryJournal::new();
        j.append(registered(1)).unwrap();
        sink.send(exec_event(0x10, 1));
        sink.record_usage([0x10; 32], "model-a", 9);
        j.append(committed(0x10)).unwrap();
        // 0x11 executes but is never committed (dead-letter shape).
        sink.send(exec_event(0x11, 1));
        sink.record_usage([0x11; 32], "model-a", 99);
        ledger.join_fold(&ReadOnly::new(j));

        let s = ledger.summarize(None).unwrap();
        assert_eq!(s.total_motes, 1, "the uncommitted execution is invisible");
        assert_eq!(s.total_output_tokens, 9);
        assert_eq!(s.rows[0].count, 1);
    }

    #[test]
    fn summary_empty_is_empty_not_fabricated() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let s = ledger.summarize(None).unwrap();
        assert!(s.rows.is_empty(), "no rows fabricated on an empty sidecar");
        assert_eq!(s.total_motes, 0);
        assert_eq!(s.total_output_tokens, 0);
    }

    /// GR10 (W1a-3) — the per-call cost of the `summarize` GROUP BY over a
    /// populated sidecar. The summary is a single table scan + group; this pins
    /// it as flat and fast even at thousands of rows (the cross-page rollup is
    /// the whole point — never a per-row client drag). Release-only.
    #[test]
    #[ignore = "GR10 measurement — meaningful in release only"]
    fn summary_query_spike() {
        let dir = tempfile::TempDir::new().unwrap();
        let ledger = TelemetryLedger::open(dir.path()).unwrap();
        let reader = GrowableReader::new();
        reader.push(registered(1), 1);
        let sink = ledger.sink();
        let total = 5_000u32;
        let models = ["m-a", "m-b", "m-c", "m-d", "m-e"];
        for i in 0..total {
            let mut id = [0u8; 32];
            id[..4].copy_from_slice(&i.to_le_bytes());
            sink.send(TelemetryEvent::Exec {
                mote_id: id,
                started_unix_ms: 1,
                wall_clock_ms: 1,
                tool_id: String::new(),
            });
            sink.record_usage(id, models[(i as usize) % models.len()], u64::from(i % 100));
            reader.push(
                JournalEntry::Committed {
                    mote_id: MoteId::from_bytes(id),
                    idempotency_key: id,
                    seq: 0,
                    nondeterminism: NdClass::Pure,
                    result_ref: ContentRef::from_bytes(id),
                    parents: SmallVec::new(),
                    warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                    mote_def_hash: MoteDefHash::from_bytes([0x09; 32]),
                },
                u64::from(i) + 2,
            );
            if i % 500 == 499 {
                ledger.join_fold(&reader);
            }
        }
        ledger.join_fold(&reader);

        let runs = 200u32;
        let t = std::time::Instant::now();
        let mut last = 0u64;
        for _ in 0..runs {
            last = ledger.summarize(None).unwrap().total_output_tokens;
        }
        let per_call = t.elapsed() / runs;
        println!("GR10 W1a-3 summarize({total} rows) per-call {per_call:?} (sum={last})");
        assert!(
            per_call < std::time::Duration::from_millis(20),
            "the GROUP BY rollup stays well under 20ms at 5k rows"
        );
    }
}
