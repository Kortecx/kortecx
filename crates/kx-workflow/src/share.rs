//! [`Manifest`] — a Delta-Sharing-style **recipe-as-product** descriptor.
//!
//! The kortecx differentiator over byte-level data sharing: a manifest shares the
//! *reproducible program* — the compiled Mote DAG (the recipe) plus its workflow
//! seed — so a recipient **regenerates byte-identical data on their own infra**
//! rather than downloading a corpus. Because [`crate::compile`] is pure and
//! deterministic and identity folds the seed (D50), the same recipe + seed yields
//! the same `MoteId`s everywhere — so a `Manifest` has a stable, content-addressed
//! [`ManifestId`] that is reproducible by reference.
//!
//! A manifest may also pin the produced corpus ([`Manifest::with_dataset`]) to
//! share a concrete result alongside the recipe. This PR defines the **format +
//! identity** only; the transport + warrant-gated auth protocol is P5-cloud.

use kx_dataset::DatasetId;
use kx_mote::MoteId;
use serde::{Deserialize, Serialize};

use crate::def::CompiledWorkflow;

/// A 32-byte content-addressed identity of a [`Manifest`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ManifestId(pub [u8; 32]);

impl ManifestId {
    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for ManifestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ManifestId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// A shareable recipe-as-product: the compiled Mote DAG (in topological order) +
/// the workflow seed that, together, regenerate byte-identical data; optionally
/// the produced corpus's [`DatasetId`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The workflow-input seed that, with the recipe, makes regeneration
    /// byte-identical (folded into entrypoint identity, D50).
    pub workflow_seed: u32,
    /// The compiled Mote DAG's `MoteId`s in topological (submission) order — the
    /// recipe a recipient re-runs to reproduce the corpus.
    pub mote_ids: Vec<MoteId>,
    /// The produced corpus, if this manifest also pins a concrete result.
    pub dataset_id: Option<DatasetId>,
}

impl Manifest {
    /// Build a recipe manifest from a compiled workflow + its seed. No corpus is
    /// pinned ([`Manifest::with_dataset`] attaches one).
    #[must_use]
    pub fn recipe(compiled: &CompiledWorkflow, workflow_seed: u32) -> Self {
        Self {
            workflow_seed,
            mote_ids: compiled.motes.iter().map(|m| m.mote.id).collect(),
            dataset_id: None,
        }
    }

    /// Pin the produced corpus to this manifest.
    #[must_use]
    pub fn with_dataset(mut self, dataset_id: DatasetId) -> Self {
        self.dataset_id = Some(dataset_id);
        self
    }

    /// The content-addressed identity — a **pure** function of seed + recipe +
    /// pinned corpus. Two byte-identical manifests share a `ManifestId`, so a
    /// recipe shared by reference is verifiable.
    #[must_use]
    pub fn id(&self) -> ManifestId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-workflow/manifest-id/v1");
        h.update(&self.workflow_seed.to_le_bytes());
        h.update(&(self.mote_ids.len() as u64).to_le_bytes());
        for mote_id in &self.mote_ids {
            h.update(mote_id.as_bytes());
        }
        match &self.dataset_id {
            Some(d) => {
                h.update(&[1]);
                h.update(&d.0);
            }
            None => {
                h.update(&[0]);
            }
        }
        ManifestId(*h.finalize().as_bytes())
    }
}
