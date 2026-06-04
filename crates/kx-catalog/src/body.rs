//! Content-addressed recipe-**body** storage (M8 / D121).
//!
//! The M7 catalog stores recipe DESCRIPTORS (signatures, snapshots,
//! advertisements). To turn "advertised" into "served", an inbound-execution
//! path must resolve the executable **body** — the [`WorkflowDef`] that compiles
//! to the recipe's Mote DAG. This module stores that body, keyed by the **same**
//! [`ManifestId`] a published [`crate::VersionedContent::Workflow`] already
//! carries, so the body links to a published version with **no schema change**.
//!
//! Discipline (mirrors [`crate::VersionLedger`] / the D-LOCK-4 grant ledger):
//! - **content-verified**: `publish_body` computes the body's `ManifestId` by
//!   [`compile`]-ing it, so a body is keyed by what it ACTUALLY compiles to — a
//!   body that does not compile to its claimed recipe cannot be stored
//!   (tamper-evident; a bait-and-switch body is impossible).
//! - **immutable + idempotent**: re-publishing the byte-identical body is a
//!   no-op; a different body that maps to the same `ManifestId` is refused.
//! - **off the trust path**: stays inside `kx-catalog` (the guarantee-path wall
//!   holds); the inbound path queries it, the spine never depends on it.

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_workflow::{compile, Manifest, ManifestId, WorkflowDef};

/// A failure publishing a recipe body.
#[derive(Debug, thiserror::Error)]
pub enum BodyLedgerError {
    /// The body does not compile (so it has no recipe identity to key on).
    #[error("recipe body does not compile: {0}")]
    Uncompilable(String),
    /// A DIFFERENT body is already published under the same recipe identity.
    #[error("a different body is already published for recipe {0} (immutability)")]
    ImmutabilityConflict(String),
    /// A durable-backend storage failure (SQLite I/O, a corrupt row, or a
    /// schema-version mismatch / content-verification failure on open).
    #[error("body-ledger storage error: {0}")]
    Storage(String),
}

/// The outcome of publishing a recipe body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyOutcome {
    /// Newly stored.
    Inserted(ManifestId),
    /// Byte-identical body already present (idempotent).
    AlreadyPresent(ManifestId),
}

/// The recipe identity a [`WorkflowDef`] compiles to — the key bodies are stored
/// under (equals the published `VersionedContent::Workflow(ManifestId)`).
///
/// # Errors
/// [`BodyLedgerError::Uncompilable`] if the body does not compile.
pub fn body_manifest_id(body: &WorkflowDef) -> Result<ManifestId, BodyLedgerError> {
    let compiled = compile(body).map_err(|e| BodyLedgerError::Uncompilable(e.to_string()))?;
    Ok(Manifest::recipe(&compiled, body.seed()).id())
}

/// A backend-agnostic store of executable recipe bodies, keyed by `ManifestId`.
pub trait BodyLedger {
    /// Store a recipe body, content-verified + immutable + idempotent. Returns
    /// the recipe's `ManifestId` (the key) and whether it was newly inserted.
    ///
    /// # Errors
    /// [`BodyLedgerError::Uncompilable`] if the body does not compile;
    /// [`BodyLedgerError::ImmutabilityConflict`] if a different body already
    /// maps to the same recipe identity.
    fn publish_body(&self, body: WorkflowDef)
        -> Result<(ManifestId, BodyOutcome), BodyLedgerError>;

    /// Resolve a recipe identity to its executable body, or `None` if absent.
    fn get_body(&self, manifest_id: &ManifestId) -> Option<WorkflowDef>;

    /// Count of stored bodies.
    fn len(&self) -> usize;

    /// `true` when no bodies are stored.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The OSS reference [`BodyLedger`]: an in-memory, content-verified map.
#[derive(Debug, Default)]
pub struct InMemoryBodyLedger {
    by_id: RwLock<BTreeMap<ManifestId, WorkflowDef>>,
}

impl InMemoryBodyLedger {
    /// An empty body store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: RwLock::new(BTreeMap::new()),
        }
    }
}

impl BodyLedger for InMemoryBodyLedger {
    fn publish_body(
        &self,
        body: WorkflowDef,
    ) -> Result<(ManifestId, BodyOutcome), BodyLedgerError> {
        let id = body_manifest_id(&body)?;
        let mut guard = self.by_id.write().expect("poisoned lock");
        match guard.get(&id) {
            Some(existing) if *existing == body => Ok((id, BodyOutcome::AlreadyPresent(id))),
            Some(_) => Err(BodyLedgerError::ImmutabilityConflict(id.to_hex())),
            None => {
                guard.insert(id, body);
                Ok((id, BodyOutcome::Inserted(id)))
            }
        }
    }

    fn get_body(&self, manifest_id: &ManifestId) -> Option<WorkflowDef> {
        self.by_id
            .read()
            .expect("poisoned lock")
            .get(manifest_id)
            .cloned()
    }

    fn len(&self) -> usize {
        self.by_id.read().expect("poisoned lock").len()
    }
}
