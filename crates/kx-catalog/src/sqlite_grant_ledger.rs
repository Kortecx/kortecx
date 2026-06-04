// SPDX-License-Identifier: Apache-2.0
//! [`SqliteGrantLedger`] — a durable [`GrantLedger`] (G1 / D94). Grants/bindings/
//! revocations survive a restart. A SQLite `facts` table (`seq` PK preserving
//! append order == in-memory `Vec` position + a content-id unique index + a
//! canonical-bincode fact BLOB) is the truth; the in-memory [`Inner`] is rebuilt
//! on open by replaying the log through the SHARED [`Inner::apply_fact`], so the
//! fold can never diverge from [`crate::InMemoryGrantLedger`]. All READ methods
//! delegate to the SAME `read_*` folds.

use std::path::Path;
use std::sync::{Mutex, RwLock};

use kx_warrant::{NarrowingError, WarrantSpec};
use rusqlite::{params, Connection, TransactionBehavior};

use crate::in_memory_ledger::{
    read_effective_grant_warrants, read_effective_grants, read_owner_of, snapshot_facts, Inner,
};
use crate::ledger::{
    AppendOutcome, AssetBinding, EffectiveGrants, GrantLedger, GrantWarrant, LedgerError,
    LedgerFact,
};
use crate::party::PartyId;
use crate::path::AssetRef;
use crate::signature::canonical_config;
use crate::sqlite_util::{open_db, open_db_in_memory};

/// The durable grant-ledger schema version.
pub const GRANT_LEDGER_SCHEMA_VERSION: u16 = 1;

const DDL: &str = "CREATE TABLE IF NOT EXISTS facts (
    seq        INTEGER PRIMARY KEY,
    fact_id    BLOB NOT NULL,
    kind       INTEGER NOT NULL,
    fact_bytes BLOB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_grant_fact_id ON facts (fact_id);";

/// A durable, SQLite-backed [`GrantLedger`].
pub struct SqliteGrantLedger {
    conn: Mutex<Connection>,
    inner: RwLock<Inner>,
}

fn store_err<E: std::fmt::Display>(err: &E) -> LedgerError {
    LedgerError::Storage(err.to_string())
}

const fn kind_of(fact: &LedgerFact) -> i64 {
    match fact {
        LedgerFact::Bind(_) => 0,
        LedgerFact::Grant(_) => 1,
        LedgerFact::Revoke(_) => 2,
    }
}

fn encode_fact(fact: &LedgerFact) -> Result<Vec<u8>, LedgerError> {
    bincode::serde::encode_to_vec(fact, canonical_config())
        .map_err(|e| LedgerError::Storage(format!("encode LedgerFact: {e}")))
}

fn next_seq(txn: &rusqlite::Transaction<'_>) -> Result<i64, LedgerError> {
    let max: Option<i64> = txn
        .query_row("SELECT MAX(seq) FROM facts", [], |r| r.get(0))
        .map_err(|e| store_err(&e))?;
    Ok(max.unwrap_or(0) + 1)
}

impl SqliteGrantLedger {
    /// Open (creating if absent) a durable grant ledger at `path`.
    ///
    /// # Errors
    /// [`LedgerError::Storage`] on a SQLite / schema / corrupt-row failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::from_conn(open_db(path, GRANT_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?)
    }

    /// Open an ephemeral in-memory durable grant ledger.
    ///
    /// # Errors
    /// [`LedgerError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, LedgerError> {
        Self::from_conn(
            open_db_in_memory(GRANT_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    fn from_conn(conn: Connection) -> Result<Self, LedgerError> {
        let inner = rebuild(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            inner: RwLock::new(inner),
        })
    }

    /// Durably append `fact` (already gated as new) under `inner`'s write lock, then
    /// replay it through the shared fold.
    fn append_durable(
        &self,
        inner: &mut Inner,
        fact: LedgerFact,
    ) -> Result<AppendOutcome, LedgerError> {
        let fid = fact.fact_id();
        let bytes = encode_fact(&fact)?;
        let mut conn = self.conn.lock().expect("poisoned mutex");
        let txn = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| store_err(&e))?;
        let seq = next_seq(&txn)?;
        txn.execute(
            "INSERT INTO facts (seq, fact_id, kind, fact_bytes) VALUES (?1, ?2, ?3, ?4)",
            params![seq, fid.as_bytes().as_slice(), kind_of(&fact), &bytes[..]],
        )
        .map_err(|e| store_err(&e))?;
        txn.commit().map_err(|e| store_err(&e))?;
        inner.apply_fact(fact);
        Ok(AppendOutcome::Appended(fid))
    }
}

/// Replay the durable log into a fresh [`Inner`] via the shared fold (append order).
fn rebuild(conn: &Connection) -> Result<Inner, LedgerError> {
    let mut inner = Inner::default();
    let mut stmt = conn
        .prepare("SELECT fact_bytes FROM facts ORDER BY seq")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let b = row.map_err(|e| store_err(&e))?;
        let (fact, _): (LedgerFact, usize) =
            bincode::serde::decode_from_slice(&b, canonical_config())
                .map_err(|e| LedgerError::Storage(format!("decode LedgerFact: {e}")))?;
        inner.apply_fact(fact);
    }
    Ok(inner)
}

impl GrantLedger for SqliteGrantLedger {
    fn append_binding(&self, binding: AssetBinding) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Bind(binding.clone());
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        // Owner conflict takes precedence (an asset has exactly one owner).
        if let Some(existing) = inner.owner_of_asset(binding.asset()) {
            if existing != binding.owner() {
                return Err(LedgerError::OwnerConflict(format!(
                    "asset {} already bound to a different owner",
                    binding.asset()
                )));
            }
            return Ok(AppendOutcome::AlreadyPresent(fid));
        }
        self.append_durable(&mut inner, fact)
    }

    fn append_grant(&self, grant: crate::grant::Grant) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Grant(Box::new(grant));
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_fact(&fid) {
            return if *existing == fact {
                Ok(AppendOutcome::AlreadyPresent(fid))
            } else {
                Err(LedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        self.append_durable(&mut inner, fact)
    }

    fn append_revocation(
        &self,
        revocation: crate::grant::Revocation,
    ) -> Result<AppendOutcome, LedgerError> {
        let fact = LedgerFact::Revoke(revocation);
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_fact(&fid) {
            return if *existing == fact {
                Ok(AppendOutcome::AlreadyPresent(fid))
            } else {
                Err(LedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        self.append_durable(&mut inner, fact)
    }

    fn owner_of(&self, asset: &AssetRef) -> Option<PartyId> {
        read_owner_of(&self.inner.read().expect("poisoned lock"), asset)
    }

    fn effective_grants(&self, party: &PartyId, asset: &AssetRef) -> EffectiveGrants {
        read_effective_grants(&self.inner.read().expect("poisoned lock"), party, asset)
    }

    fn effective_grant_warrants(
        &self,
        party: &PartyId,
        asset: &AssetRef,
        owner_root: &WarrantSpec,
    ) -> Result<Vec<GrantWarrant>, NarrowingError> {
        read_effective_grant_warrants(
            &self.inner.read().expect("poisoned lock"),
            party,
            asset,
            owner_root,
        )
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = LedgerFact> + 'a> {
        let facts = snapshot_facts(&self.inner.read().expect("poisoned lock"));
        Box::new(facts.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").len_facts()
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteGrantLedger>();
};
