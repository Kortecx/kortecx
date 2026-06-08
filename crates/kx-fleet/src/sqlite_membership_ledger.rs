// SPDX-License-Identifier: Apache-2.0
//! [`SqliteMembershipLedger`] — a durable [`MembershipLedger`] (G1a / D94).
//! Foundings/admits/removals/disbands survive a restart. A SQLite `facts` table
//! (`seq` PK preserving append order == in-memory `Vec` position + a content-id
//! unique index + a canonical-bincode fact BLOB) is the truth; the in-memory
//! [`Inner`] is rebuilt on open by replaying the log through the SHARED
//! [`Inner::apply_fact`], so the fold can never diverge from
//! [`crate::InMemoryMembershipLedger`]. All READ methods delegate to the SAME
//! `read_*` folds. Mirrors `kx_catalog::SqliteGrantLedger`.
//!
//! ## Why `seq` position matters here (more than for the grant ledger)
//!
//! The membership fold is **time-ordered**: a [`crate::Removal`] cancels only the
//! admits that PRECEDE it (a re-admit appended later survives), and that ordering is
//! the `facts` position. The `seq INTEGER PRIMARY KEY` preserves the exact append
//! order across a restart, so the replayed positions reproduce the live fold's
//! removal/re-admit semantics byte-for-byte.

use std::path::Path;
use std::sync::{Mutex, RwLock};

use kx_catalog::{canonical_config, PartyId};
use rusqlite::{params, Connection, TransactionBehavior};

use crate::error::MembershipLedgerError;
use crate::ledger::{MemberRole, MembershipLedger, MembershipOutcome, TeamEdge};
use crate::membership::{Admit, Disband, MembershipFact, MembershipId, Removal};
use crate::membership_inner::{
    read_effective_members, read_member_edges, read_owner_of_team, snapshot_facts, Inner,
};
use crate::sqlite_util::{open_db, open_db_in_memory};
use crate::team::Team;

/// The durable membership-ledger schema version.
pub const MEMBERSHIP_LEDGER_SCHEMA_VERSION: u16 = 1;

const DDL: &str = "CREATE TABLE IF NOT EXISTS facts (
    seq        INTEGER PRIMARY KEY,
    fact_id    BLOB NOT NULL,
    kind       INTEGER NOT NULL,
    fact_bytes BLOB NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_membership_fact_id ON facts (fact_id);";

/// A durable, SQLite-backed [`MembershipLedger`].
pub struct SqliteMembershipLedger {
    conn: Mutex<Connection>,
    inner: RwLock<Inner>,
}

fn store_err<E: std::fmt::Display>(err: &E) -> MembershipLedgerError {
    MembershipLedgerError::Storage(err.to_string())
}

const fn kind_of(fact: &MembershipFact) -> i64 {
    match fact {
        MembershipFact::Found(_) => 0,
        MembershipFact::Admit(_) => 1,
        MembershipFact::Remove(_) => 2,
        MembershipFact::Disband(_) => 3,
    }
}

fn encode_fact(fact: &MembershipFact) -> Result<Vec<u8>, MembershipLedgerError> {
    bincode::serde::encode_to_vec(fact, canonical_config())
        .map_err(|e| MembershipLedgerError::Storage(format!("encode MembershipFact: {e}")))
}

fn next_seq(txn: &rusqlite::Transaction<'_>) -> Result<i64, MembershipLedgerError> {
    let max: Option<i64> = txn
        .query_row("SELECT MAX(seq) FROM facts", [], |r| r.get(0))
        .map_err(|e| store_err(&e))?;
    Ok(max.unwrap_or(0) + 1)
}

impl SqliteMembershipLedger {
    /// Open (creating if absent) a durable membership ledger at `path`.
    ///
    /// # Errors
    /// [`MembershipLedgerError::Storage`] on a SQLite / schema / corrupt-row failure.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MembershipLedgerError> {
        Self::from_conn(
            open_db(path, MEMBERSHIP_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    /// Open an ephemeral in-memory durable membership ledger.
    ///
    /// # Errors
    /// [`MembershipLedgerError::Storage`] on a SQLite failure.
    pub fn open_in_memory() -> Result<Self, MembershipLedgerError> {
        Self::from_conn(
            open_db_in_memory(MEMBERSHIP_LEDGER_SCHEMA_VERSION, DDL).map_err(|e| store_err(&e))?,
        )
    }

    fn from_conn(conn: Connection) -> Result<Self, MembershipLedgerError> {
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
        fact: MembershipFact,
    ) -> Result<MembershipOutcome, MembershipLedgerError> {
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
        Ok(MembershipOutcome::Appended(fid))
    }
}

/// Replay the durable log into a fresh [`Inner`] via the shared fold (append order).
fn rebuild(conn: &Connection) -> Result<Inner, MembershipLedgerError> {
    let mut inner = Inner::default();
    let mut stmt = conn
        .prepare("SELECT fact_bytes FROM facts ORDER BY seq")
        .map_err(|e| store_err(&e))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .map_err(|e| store_err(&e))?;
    for row in rows {
        let b = row.map_err(|e| store_err(&e))?;
        let (fact, _): (MembershipFact, usize) =
            bincode::serde::decode_from_slice(&b, canonical_config()).map_err(|e| {
                MembershipLedgerError::Storage(format!("decode MembershipFact: {e}"))
            })?;
        inner.apply_fact(fact);
    }
    Ok(inner)
}

impl MembershipLedger for SqliteMembershipLedger {
    fn append_founding(&self, team: Team) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fid = MembershipId::from_bytes(*team.team_id().as_bytes());
        let team_principal = team.team().clone();
        let owner = team.owner().clone();
        let mut inner = self.inner.write().expect("poisoned lock");
        // Genesis is set ONCE per team principal (owner-conflict gate); a re-founding
        // by the same owner is idempotent (first display name wins, byte-identical
        // re-foundings collapse) — the gate runs BEFORE the durable INSERT, so only a
        // fresh founding is ever persisted (the replayed log carries one Found/team).
        if let Some(existing_owner) = inner.owner_of_team_principal(&team_principal) {
            if existing_owner != &owner {
                return Err(MembershipLedgerError::OwnerConflict(format!(
                    "team {team_principal} already founded with a different owner"
                )));
            }
            let canonical = inner.canonical_founding(&team_principal).unwrap_or(fid);
            return Ok(MembershipOutcome::AlreadyPresent(canonical));
        }
        self.append_durable(&mut inner, MembershipFact::Found(Box::new(team)))
    }

    fn append_admit(&self, admit: Admit) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Admit(Box::new(admit));
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        self.append_durable(&mut inner, fact)
    }

    fn append_remove(&self, removal: Removal) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Remove(Box::new(removal));
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        self.append_durable(&mut inner, fact)
    }

    fn append_disband(&self, disband: Disband) -> Result<MembershipOutcome, MembershipLedgerError> {
        let fact = MembershipFact::Disband(Box::new(disband));
        let fid = fact.fact_id();
        let mut inner = self.inner.write().expect("poisoned lock");
        if let Some(existing) = inner.contains_fact(&fid) {
            return if *existing == fact {
                Ok(MembershipOutcome::AlreadyPresent(fid))
            } else {
                Err(MembershipLedgerError::ImmutabilityConflict(fid.to_hex()))
            };
        }
        self.append_durable(&mut inner, fact)
    }

    fn owner_of_team(&self, team: &PartyId) -> Option<PartyId> {
        read_owner_of_team(&self.inner.read().expect("poisoned lock"), team)
    }

    fn member_edges(&self, member: &PartyId) -> Vec<TeamEdge> {
        read_member_edges(&self.inner.read().expect("poisoned lock"), member)
    }

    fn effective_members(&self, team: &PartyId) -> Vec<MemberRole> {
        read_effective_members(&self.inner.read().expect("poisoned lock"), team)
    }

    fn list_facts<'a>(&'a self) -> Box<dyn Iterator<Item = MembershipFact> + 'a> {
        let facts = snapshot_facts(&self.inner.read().expect("poisoned lock"));
        Box::new(facts.into_iter())
    }

    fn len(&self) -> usize {
        self.inner.read().expect("poisoned lock").len_facts()
    }
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SqliteMembershipLedger>();
};
