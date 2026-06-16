//! The alerts-inbox read-cache — durable serve-path projection of terminal
//! failures (W1a-2, the observability/operability "Alerts" surface).
//!
//! `AlertsDb` is a background poll-fold of the gateway's read-only journal handle
//! into a durable `alerts.db` (under `--catalog-dir`), exposing the operator
//! alerts inbox through the `ListAlerts` RPC. It mirrors the `capture.db` posture
//! exactly (a journal-DERIVED, rebuildable cache; the coordinator stays the sole
//! writer; zero latency on the commit path; the canonical projection digest is
//! untouched). On open it reconciles against the journal: a `schema_version`
//! mismatch or a corrupt/torn sidecar ⇒ drop-and-rebuild from seq 0; otherwise it
//! resumes incrementally from the durable watermark.
//!
//! ## What folds (the scope, load-bearing)
//! Only the journal's TERMINAL `Failed` facts (`!is_pre_commit_crash` — the
//! single source of class truth in `kx-journal`): dead-letters (F4) +
//! worker-reported terminal failures. The liveness pre-commit crashes
//! (`TimedOut`/`WorkerCrashed`) are EXCLUDED — they re-dispatch, they are not an
//! alert. Serve-path admission refusals write NOTHING to the journal (they are
//! synchronous `SUBMIT_STATUS_REJECTED` responses carrying `kx-refusal-code`
//! metadata), so they are not foldable and not in this inbox.
//!
//! ## Identity (re-fold-stable)
//! `alert_id = blake3("kx-alert-id\0" ‖ mote_id ‖ seq_le)[..16]`. `Failed` is NOT
//! dedup-by-key (many attempts per mote), so the journal `seq` disambiguates
//! retries; the derivation is deterministic, so deleting `alerts.db` and
//! re-folding re-materializes the SAME `alert_id` set (`INSERT OR IGNORE` is
//! idempotent). This is the HARD-gate property: derived, never stored.
//!
//! ## OSS = the read-only VIEW
//! There is no acknowledge/resolve mutation here — the triage lifecycle, the
//! alert-rule engine, and outbound notifications are a CLOUD capability
//! (D156 / D129; GR19).

use std::path::Path;
use std::sync::Mutex;

use kx_gateway_core::GatewayError as CoreError;
use kx_gateway_core::{AlertEntry, AlertView, JournalReader};
use kx_journal::{is_pre_commit_crash, FailureReason, JournalEntry};
use rusqlite::{params, Connection};

use crate::error::GatewayError;

/// The alerts-projection schema version. A bump (or any decode failure) makes the
/// load path drop-and-rebuild from the journal — alerts are a cache, so there is
/// NEVER a migration, only a rebuild.
const SCHEMA_VERSION: i64 = 1;

/// The durable schema (idempotent). `alerts` is the terminal-failure projection;
/// `run_meta` holds the single-node run instance id (durable across fold ticks);
/// `meta` holds the `last_seq` fold watermark + the `schema_version`.
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS alerts (
    alert_id        BLOB PRIMARY KEY,
    mote_id         BLOB NOT NULL,
    instance_id     BLOB NOT NULL,
    reason_class    TEXT NOT NULL,
    reason_code     INTEGER NOT NULL,
    severity        TEXT NOT NULL,
    seq             INTEGER NOT NULL,
    created_unix_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS alerts_by_seq ON alerts(seq DESC);
CREATE INDEX IF NOT EXISTS alerts_by_instance ON alerts(instance_id, seq DESC);
-- The run instance id (single-node: one RunRegistered per journal) is DURABLE
-- across fold ticks, so an alert folded in a LATER tick is still stamped even
-- though RunRegistered (seq=1) folded in an EARLIER tick. A blob, so it cannot
-- live in `meta` (INTEGER values).
CREATE TABLE IF NOT EXISTS run_meta (id INTEGER PRIMARY KEY CHECK (id = 0), instance_id BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);";

/// The closed `FailureReason` wire vocabulary (a string, so a future variant is
/// additive on the wire — the `nd_class`/`branch` capture precedent).
fn reason_wire(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::TimedOut => "timed_out",
        FailureReason::ExecutorRefused => "executor_refused",
        FailureReason::ValidatorRejected => "validator_rejected",
        FailureReason::WorkerCrashed => "worker_crashed",
        FailureReason::UpstreamRepudiated => "upstream_repudiated",
        FailureReason::UnsafeWorldMutatingConstruction => "unsafe_world_mutating_construction",
        FailureReason::CompensatedAtLeastOnce => "compensated_at_least_once",
        FailureReason::QuarantinedAtLeastOnce => "quarantined_at_least_once",
        FailureReason::DeadLettered => "dead_lettered",
    }
}

/// The closed display-severity vocabulary. Deliberate refusals (an unsafe
/// construction, an executor refusal) read as `"refused"`; every other terminal
/// failure reads as `"error"`.
fn severity_for(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::ExecutorRefused | FailureReason::UnsafeWorldMutatingConstruction => {
            "refused"
        }
        _ => "error",
    }
}

/// The server-derived, re-fold-stable alert id (SN-8: the client can neither name
/// nor forge it). Deterministic over `(mote_id, seq)` so a re-fold of the same
/// `Failed` fact maps to the same id.
fn alert_id_for(mote_id: &[u8; 32], seq: u64) -> [u8; 16] {
    let mut keyed = Vec::with_capacity(12 + 32 + 8);
    keyed.extend_from_slice(b"kx-alert-id\0");
    keyed.extend_from_slice(mote_id);
    keyed.extend_from_slice(&seq.to_le_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&kx_content::ContentRef::of(&keyed).0[..16]);
    id
}

/// The durable alerts-projection ledger over `alerts.db`. A single mutex'd
/// connection serves both the periodic fold (write) and the `ListAlerts` read —
/// alerts are low-traffic (one page read; a periodic incremental fold), so the
/// mutex is never contended.
pub(crate) struct AlertsDb {
    conn: Mutex<Connection>,
}

impl AlertsDb {
    /// Open (or create) `alerts.db` under `dir`, reconcile against the journal,
    /// and return the ledger. A `schema_version` mismatch or a corrupt sidecar
    /// drops every table (the watermark resets to 0 ⇒ the next fold backfills
    /// from the journal). Idempotent on restart.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] on an unrecoverable open/pragma failure.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| GatewayError::Catalog(format!("alerts dir: {e}")))?;
        let db_path = dir.join("alerts.db");
        // A CORRUPT/foreign file (not a SQLite database) makes even the pragma
        // fail. Alerts are a rebuildable cache (the journal is truth), so we
        // delete the unreadable file(s) and recreate (the next fold backfills
        // from seq 0). A valid-but-stale-schema DB is handled below.
        let conn = if let Ok(c) = Self::open_with_pragma(&db_path) {
            c
        } else {
            let _ = std::fs::remove_file(&db_path);
            let _ = std::fs::remove_file(dir.join("alerts.db-wal"));
            let _ = std::fs::remove_file(dir.join("alerts.db-shm"));
            Self::open_with_pragma(&db_path)
                .map_err(|e| GatewayError::Catalog(format!("alerts reopen: {e}")))?
        };
        let fresh_or_stale = match Self::read_schema_version(&conn) {
            Ok(Some(v)) => v != SCHEMA_VERSION,
            Ok(None) | Err(_) => true,
        };
        if fresh_or_stale {
            conn.execute_batch(
                "DROP TABLE IF EXISTS alerts;
                 DROP TABLE IF EXISTS run_meta;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(|e| GatewayError::Catalog(format!("alerts rebuild: {e}")))?;
        }
        conn.execute_batch(SCHEMA)
            .map_err(|e| GatewayError::Catalog(format!("alerts schema: {e}")))?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?1), ('last_seq', 0)",
            params![SCHEMA_VERSION],
        )
        .map_err(|e| GatewayError::Catalog(format!("alerts meta init: {e}")))?;
        Ok(Self {
            conn: Mutex::new(conn),
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

    /// The durable fold watermark (the highest seq folded into the projection).
    fn watermark(conn: &Connection) -> i64 {
        conn.query_row("SELECT value FROM meta WHERE key = 'last_seq'", [], |r| {
            r.get(0)
        })
        .unwrap_or(0)
    }

    /// Incrementally fold `(watermark, head]` of `reader` into the projection:
    /// insert each TERMINAL `Failed` fact as an alert row (`INSERT OR IGNORE` by
    /// the re-fold-stable `alert_id`), stamping the session instance id. One
    /// transaction per tick; advancing the watermark last. A read/store fault is
    /// logged and skipped (the next tick retries — fail-safe, never panics on the
    /// background task). Returns the number of alert rows inserted this tick.
    pub(crate) fn fold(&self, reader: &dyn JournalReader) -> usize {
        let head = match reader.current_seq() {
            Ok(h) => h,
            Err(error) => {
                tracing::warn!(%error, "alerts fold: journal head read failed");
                return 0;
            }
        };
        let Ok(mut conn) = self.conn.lock() else {
            return 0;
        };
        let from = u64::try_from(Self::watermark(&conn)).unwrap_or(0);
        if head <= from {
            return 0; // nothing new
        }
        let entries = match reader.read_entries_by_seq(from + 1..head.saturating_add(1)) {
            Ok(e) => e,
            Err(error) => {
                tracing::warn!(%error, "alerts fold: range read failed");
                return 0;
            }
        };
        let tx = match conn.transaction() {
            Ok(t) => t,
            Err(error) => {
                tracing::warn!(%error, "alerts fold: begin txn failed");
                return 0;
            }
        };
        let mut inserted = 0usize;
        // The serve session's instance id (single-node: one RunRegistered per
        // journal). Seeded from the DURABLE run_meta row — so an alert folded in a
        // later tick is stamped even though RunRegistered (seq=1) folded earlier.
        let mut instance: Option<Vec<u8>> = Self::run_instance(&tx).ok().flatten();
        let now_ms = now_unix_ms();
        for entry in entries {
            match entry {
                JournalEntry::RunRegistered { instance_id, .. } => {
                    instance = Some(instance_id.to_vec());
                    let _ = tx.execute(
                        "INSERT OR REPLACE INTO run_meta(id, instance_id) VALUES (0, ?1)",
                        params![instance_id.to_vec()],
                    );
                }
                JournalEntry::Failed {
                    mote_id,
                    seq,
                    reason_class,
                    ..
                } => {
                    // TERMINAL failures only — the liveness pre-commit crashes
                    // (`TimedOut`/`WorkerCrashed`) re-dispatch, they do not alert.
                    if is_pre_commit_crash(reason_class) {
                        continue;
                    }
                    let mote = *mote_id.as_bytes();
                    let alert_id = alert_id_for(&mote, seq);
                    if tx
                        .execute(
                            "INSERT OR IGNORE INTO alerts \
                             (alert_id, mote_id, instance_id, reason_class, reason_code, severity, seq, created_unix_ms) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![
                                alert_id.to_vec(),
                                mote.to_vec(),
                                instance.clone().unwrap_or_default(),
                                reason_wire(reason_class),
                                i64::from(reason_class.as_u8()),
                                severity_for(reason_class),
                                i64::try_from(seq).unwrap_or(i64::MAX),
                                i64::try_from(now_ms).unwrap_or(i64::MAX),
                            ],
                        )
                        .map_or(0, |n| n)
                        > 0
                    {
                        inserted += 1;
                    }
                }
                _ => {}
            }
        }
        // Stamp every still-unstamped alert with the session instance id (the
        // registration is seq=1, usually folded first, but be order-robust).
        if let Some(inst) = &instance {
            let _ = tx.execute(
                "UPDATE alerts SET instance_id = ?1 WHERE length(instance_id) <> 16",
                params![inst],
            );
        }
        let _ = tx.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('last_seq', ?1)",
            params![i64::try_from(head).unwrap_or(i64::MAX)],
        );
        if let Err(error) = tx.commit() {
            tracing::warn!(%error, "alerts fold: commit failed (retried next tick)");
            return 0;
        }
        inserted
    }

    /// The serve session's instance id from the DURABLE `run_meta` row, or `None`
    /// before any run registered.
    fn run_instance(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<Option<Vec<u8>>> {
        tx.query_row("SELECT instance_id FROM run_meta WHERE id = 0", [], |r| {
            r.get(0)
        })
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
    }
}

impl AlertView for AlertsDb {
    fn list(
        &self,
        limit: usize,
        instance_id: Option<[u8; 16]>,
        before_seq: Option<u64>,
    ) -> Result<(Vec<AlertEntry>, bool), CoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| CoreError::Internal("alerts lock poisoned".into()))?;
        // Over-fetch by one to compute `has_more` without a second COUNT query.
        let over = i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX);
        // The cursor: rows with seq strictly below `before_seq` (newest-first);
        // absent ⇒ from the head. i64::MAX is above any real seq.
        let cursor = before_seq
            .map_or(i64::MAX, |s| i64::try_from(s).unwrap_or(i64::MAX))
            .saturating_sub(1);
        let mut rows: Vec<AlertEntry> = match instance_id {
            Some(inst) => Self::select(
                &conn,
                "SELECT alert_id, mote_id, instance_id, reason_class, reason_code, severity, seq, created_unix_ms \
                 FROM alerts WHERE instance_id = ?1 AND seq <= ?2 ORDER BY seq DESC LIMIT ?3",
                params![inst.to_vec(), cursor, over],
            ),
            None => Self::select(
                &conn,
                "SELECT alert_id, mote_id, instance_id, reason_class, reason_code, severity, seq, created_unix_ms \
                 FROM alerts WHERE seq <= ?1 ORDER BY seq DESC LIMIT ?2",
                params![cursor, over],
            ),
        }?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Ok((rows, has_more))
    }
}

impl AlertsDb {
    fn select(
        conn: &Connection,
        sql: &str,
        p: impl rusqlite::Params,
    ) -> Result<Vec<AlertEntry>, CoreError> {
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| CoreError::Internal(format!("alerts query prep: {e}")))?;
        let rows = stmt
            .query_map(p, |r| {
                let alert: Vec<u8> = r.get(0)?;
                let mote: Vec<u8> = r.get(1)?;
                let inst: Vec<u8> = r.get(2)?;
                let reason_class: String = r.get(3)?;
                let reason_code: i64 = r.get(4)?;
                let severity: String = r.get(5)?;
                let seq: i64 = r.get(6)?;
                let created: i64 = r.get(7)?;
                Ok((
                    alert,
                    mote,
                    inst,
                    reason_class,
                    reason_code,
                    severity,
                    seq,
                    created,
                ))
            })
            .map_err(|e| CoreError::Internal(format!("alerts query: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (alert, mote, inst, reason_class, reason_code, severity, seq, created) =
                row.map_err(|e| CoreError::Internal(format!("alerts row: {e}")))?;
            out.push(AlertEntry {
                alert_id: <[u8; 16]>::try_from(alert.as_slice()).unwrap_or([0; 16]),
                mote_id: <[u8; 32]>::try_from(mote.as_slice()).unwrap_or([0; 32]),
                instance_id: <[u8; 16]>::try_from(inst.as_slice()).unwrap_or([0; 16]),
                reason_class,
                reason_code: u32::try_from(reason_code).unwrap_or(0),
                severity,
                seq: u64::try_from(seq).unwrap_or(0),
                created_unix_ms: u64::try_from(created).unwrap_or(0),
            });
        }
        Ok(out)
    }
}

/// Audit-only wall clock (ms since epoch); off every hash.
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use kx_gateway_core::ReadOnly;
    use kx_journal::{InMemoryJournal, Journal};
    use kx_mote::MoteId;

    use super::*;

    fn failed(seq: u64, mote: u8, reason: FailureReason) -> JournalEntry {
        JournalEntry::Failed {
            mote_id: MoteId::from_bytes([mote; 32]),
            idempotency_key: [mote; 32],
            seq,
            reason_class: reason,
            reporter_id: 0,
        }
    }

    fn run_registered(seq: u64, instance: u8) -> JournalEntry {
        JournalEntry::RunRegistered {
            instance_id: [instance; 16],
            recipe_fingerprint: [0xab; 32],
            ts: 0,
            seq,
        }
    }

    /// A journal seeded with the given entries (seq is taken from each entry).
    fn journal_with(entries: &[JournalEntry]) -> ReadOnly<InMemoryJournal> {
        let j = InMemoryJournal::new();
        for e in entries {
            j.append(e.clone()).expect("append");
        }
        ReadOnly::new(j)
    }

    fn db() -> (AlertsDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = AlertsDb::open(dir.path()).expect("open");
        (db, dir)
    }

    #[test]
    fn folds_only_terminal_failures() {
        let (db, _g) = db();
        let j = journal_with(&[
            run_registered(1, 7),
            failed(2, 0x10, FailureReason::TimedOut), // liveness — EXCLUDED
            failed(3, 0x11, FailureReason::WorkerCrashed), // liveness — EXCLUDED
            failed(4, 0x12, FailureReason::DeadLettered), // terminal — included
            failed(5, 0x13, FailureReason::ValidatorRejected), // terminal — included
            failed(6, 0x14, FailureReason::UnsafeWorldMutatingConstruction), // terminal — included
        ]);
        let n = db.fold(&j);
        assert_eq!(n, 3, "only the 3 terminal failures alert");
        let (rows, has_more) = db.list(100, None, None).expect("list");
        assert!(!has_more);
        assert_eq!(rows.len(), 3);
        // newest-first.
        assert_eq!(rows[0].seq, 6);
        assert_eq!(rows[0].reason_class, "unsafe_world_mutating_construction");
        assert_eq!(rows[0].severity, "refused");
        assert_eq!(rows[1].severity, "error"); // validator_rejected
        assert_eq!(rows[2].reason_class, "dead_lettered");
        // every alert stamped with the session instance.
        assert!(rows.iter().all(|r| r.instance_id == [7; 16]));
    }

    #[test]
    fn terminal_filter_matches_is_pre_commit_crash_for_every_variant() {
        // The fold filter MUST be exactly `!is_pre_commit_crash` — a sweep over
        // all 9 variants so a future variant is forced through the classifier.
        let variants = [
            FailureReason::TimedOut,
            FailureReason::ExecutorRefused,
            FailureReason::ValidatorRejected,
            FailureReason::WorkerCrashed,
            FailureReason::UpstreamRepudiated,
            FailureReason::UnsafeWorldMutatingConstruction,
            FailureReason::CompensatedAtLeastOnce,
            FailureReason::QuarantinedAtLeastOnce,
            FailureReason::DeadLettered,
        ];
        for (i, &reason) in variants.iter().enumerate() {
            let (db, _g) = db();
            let seq = (i as u64) + 2;
            let j = journal_with(&[run_registered(1, 1), failed(seq, 0x20, reason)]);
            let n = db.fold(&j);
            let expected = usize::from(!is_pre_commit_crash(reason));
            assert_eq!(n, expected, "variant {reason:?} alert-count");
        }
    }

    #[test]
    fn ignores_non_failed_kinds() {
        let (db, _g) = db();
        let j = journal_with(&[run_registered(1, 3)]);
        assert_eq!(db.fold(&j), 0);
        let (rows, _) = db.list(100, None, None).expect("list");
        assert!(rows.is_empty());
    }

    #[test]
    fn empty_journal_is_empty() {
        let (db, _g) = db();
        let j = journal_with(&[]);
        assert_eq!(db.fold(&j), 0);
        let (rows, has_more) = db.list(100, None, None).expect("list");
        assert!(rows.is_empty());
        assert!(!has_more);
    }

    #[test]
    fn fold_is_idempotent_and_watermarked() {
        let (db, _g) = db();
        let j = journal_with(&[
            run_registered(1, 5),
            failed(2, 0x30, FailureReason::DeadLettered),
        ]);
        assert_eq!(db.fold(&j), 1);
        assert_eq!(db.fold(&j), 0, "re-fold past the watermark is a no-op");
        let (rows, _) = db.list(100, None, None).expect("list");
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn alert_id_is_deterministic_across_refold() {
        // The HARD-gate property: same (mote_id, seq) ⇒ same alert_id ⇒ a full
        // rebuild re-materializes the SAME item set.
        let entries = [
            run_registered(1, 9),
            failed(2, 0x40, FailureReason::DeadLettered),
            failed(3, 0x41, FailureReason::UpstreamRepudiated),
        ];
        let (db1, _g1) = db();
        db1.fold(&journal_with(&entries));
        let (rows1, _) = db1.list(100, None, None).expect("list1");
        // A fresh DB (simulating a delete + rebuild) over the SAME journal.
        let (db2, _g2) = db();
        db2.fold(&journal_with(&entries));
        let (rows2, _) = db2.list(100, None, None).expect("list2");
        let ids1: Vec<_> = rows1.iter().map(|r| r.alert_id).collect();
        let ids2: Vec<_> = rows2.iter().map(|r| r.alert_id).collect();
        assert_eq!(ids1, ids2, "the alert item set is re-fold-stable");
        assert!(!ids1.is_empty());
    }

    #[test]
    fn pagination_has_more_and_cursor() {
        let (db, _g) = db();
        let mut entries = vec![run_registered(1, 2)];
        for i in 0..5u8 {
            entries.push(failed(
                u64::from(i) + 2,
                0x50 + i,
                FailureReason::DeadLettered,
            ));
        }
        db.fold(&journal_with(&entries));
        let (page1, has_more) = db.list(2, None, None).expect("page1");
        assert_eq!(page1.len(), 2);
        assert!(has_more);
        assert_eq!(page1[0].seq, 6); // newest first
        let (page2, _) = db.list(2, None, Some(page1[1].seq)).expect("page2");
        assert!(
            page2[0].seq < page1[1].seq,
            "cursor advances strictly below"
        );
    }

    #[test]
    fn deleting_the_db_then_refolding_remateralizes_the_same_items() {
        // THE HARD GATE (the file-delete form): fold real terminal Failed facts,
        // delete alerts.db, reopen + re-fold the SAME journal ⇒ the SAME alert
        // item set re-materializes (derived from committed facts, not stored). The
        // lifecycle would reset, but OSS has none — the item set is the contract.
        let dir = tempfile::tempdir().expect("tempdir");
        let entries = [
            run_registered(1, 9),
            failed(2, 0x70, FailureReason::DeadLettered),
            failed(3, 0x71, FailureReason::TimedOut), // liveness — never an item
            failed(4, 0x72, FailureReason::ValidatorRejected),
        ];
        let ids_before: Vec<[u8; 16]> = {
            let db = AlertsDb::open(dir.path()).expect("open");
            db.fold(&journal_with(&entries));
            let (rows, _) = db.list(100, None, None).expect("list");
            rows.iter().map(|r| r.alert_id).collect()
        };
        // Delete the sidecar files (+WAL/SHM) — the operator's `rm alerts.db`.
        std::fs::remove_file(dir.path().join("alerts.db")).expect("rm");
        let _ = std::fs::remove_file(dir.path().join("alerts.db-wal"));
        let _ = std::fs::remove_file(dir.path().join("alerts.db-shm"));
        let ids_after: Vec<[u8; 16]> = {
            let db = AlertsDb::open(dir.path()).expect("reopen");
            db.fold(&journal_with(&entries));
            let (rows, _) = db.list(100, None, None).expect("list");
            rows.iter().map(|r| r.alert_id).collect()
        };
        assert_eq!(ids_before, ids_after, "same item set re-materializes");
        assert_eq!(ids_before.len(), 2, "2 terminal alerts (TimedOut excluded)");
    }

    #[test]
    fn schema_drift_rebuilds_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let db = AlertsDb::open(dir.path()).expect("open");
            db.fold(&journal_with(&[
                run_registered(1, 1),
                failed(2, 0x60, FailureReason::DeadLettered),
            ]));
        }
        // Bump the on-disk schema_version → the next open drops + rebuilds.
        {
            let conn = Connection::open(dir.path().join("alerts.db")).expect("reopen");
            conn.execute(
                "UPDATE meta SET value = 999 WHERE key = 'schema_version'",
                [],
            )
            .expect("bump");
        }
        let db = AlertsDb::open(dir.path()).expect("reopen ledger");
        let (rows, _) = db.list(100, None, None).expect("list");
        assert!(
            rows.is_empty(),
            "a stale schema rebuilds to empty (re-fold required)"
        );
    }

    /// GR10 spike (release, `--ignored`): per-entry fold cost + a list read over a
    /// large inbox. `cargo test -p kx-gateway --release fold_spike -- --ignored --nocapture`.
    #[test]
    #[ignore = "perf spike — run explicitly with --release --ignored --nocapture"]
    fn fold_spike() {
        use std::time::Instant;
        const N: u64 = 50_000;
        let mut entries = Vec::with_capacity(usize::try_from(N).unwrap_or(0) + 1);
        entries.push(run_registered(1, 1));
        for i in 0..N {
            // Alternate terminal/liveness so the filter does real work.
            let reason = if i % 2 == 0 {
                FailureReason::DeadLettered
            } else {
                FailureReason::TimedOut
            };
            entries.push(failed(i + 2, u8::try_from(i % 251).unwrap_or(0), reason));
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let db = AlertsDb::open(dir.path()).expect("open");
        let reader = journal_with(&entries);

        let t0 = Instant::now();
        let inserted = db.fold(&reader);
        let fold = t0.elapsed();

        let t1 = Instant::now();
        let (rows, has_more) = db.list(200, None, None).expect("list");
        let read = t1.elapsed();

        let per_entry_ns = fold.as_nanos() / u128::from(N);
        println!(
            "alerts fold_spike: N={N} → {inserted} alerts fold={fold:?} ({per_entry_ns} ns/entry) \
             · list(200)={read:?} has_more={has_more}"
        );
        assert_eq!(
            inserted,
            usize::try_from(N / 2).unwrap_or(0),
            "only terminal failures alert"
        );
        assert_eq!(rows.len(), 200);
    }
}
