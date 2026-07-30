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

use kx_content::ContentRef;
use kx_dataset::DatasetId;
use kx_mote::MoteId;
use kx_warrant::warrant_ref_of;
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
    /// Each compiled step's `warrant_ref`, in the SAME topological order as
    /// [`Self::mote_ids`] — the AUTHORITY half of the recipe.
    ///
    /// A step warrant is not part of `MoteDef`, so it never reaches `MoteId`
    /// (that is correct: two runs differing only in authority are the same
    /// computation). But a recipe is a *program plus the authority it runs
    /// under*, and the body ledger keys bodies by `ManifestId` and refuses a
    /// different body under an existing id. Without this field, changing a
    /// react warrant — the served model's granted tool set, a decode budget —
    /// produced different body BYTES under an UNCHANGED id, and
    /// `SqliteBodyLedger::publish_body` refused it as an immutability
    /// violation. That refusal is on `open_serve`'s startup path, so the
    /// upgraded binary failed the SERVE BOOT of every already-seeded state dir.
    ///
    /// Folding the warrants in makes a warrant change a genuinely different
    /// recipe with its own id, so the ledger stores it beside the old one
    /// instead of rejecting it — **without** relaxing the immutability rule,
    /// which is what stops a replacement body WIDENING a recipe's authority
    /// under an unchanged id.
    pub step_warrant_refs: Vec<ContentRef>,
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
            step_warrant_refs: compiled
                .motes
                .iter()
                .map(|m| warrant_ref_of(&m.warrant))
                .collect(),
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
    /// step authority + pinned corpus. Two byte-identical manifests share a
    /// `ManifestId`, so a recipe shared by reference is verifiable.
    ///
    /// **Domain tag `…/v2`** (was `v1`): the tag covers `step_warrant_refs`, so
    /// every recipe id moves once, by construction. That is intended and is why
    /// this change ships alone — see [`Manifest::step_warrant_refs`]. The tag is
    /// bumped rather than reused so a `v1` id can never alias a `v2` one.
    #[must_use]
    pub fn id(&self) -> ManifestId {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-workflow/manifest-id/v2");
        h.update(&self.workflow_seed.to_le_bytes());
        h.update(&(self.mote_ids.len() as u64).to_le_bytes());
        for mote_id in &self.mote_ids {
            h.update(mote_id.as_bytes());
        }
        // Length-prefixed and separately folded: `mote_ids` and
        // `step_warrant_refs` are both 32-byte sequences, so concatenating them
        // unprefixed would let a (n+1, m-1) split alias an (n, m) one.
        h.update(&(self.step_warrant_refs.len() as u64).to_le_bytes());
        for warrant_ref in &self.step_warrant_refs {
            h.update(warrant_ref.as_bytes());
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

    /// The **superseded** `…/v1` identity: seed + Mote ids + pinned corpus, with no
    /// step authority folded in.
    ///
    /// Read-only, and deliberately retained rather than deleted. A durable body ledger
    /// written by an older binary holds rows KEYED by this scheme, and its open path
    /// re-derives every stored key to prove the row was not tampered with. If the only
    /// derivation available were [`Self::id`], every one of those rows would fail
    /// verification and the ledger would refuse to open — turning the boot failure this
    /// change exists to fix into a different boot failure on the same state dirs. So a
    /// version that was ever admitted keeps its rung on the ladder, and the verifier
    /// accepts a row that matches EITHER scheme.
    ///
    /// This is not a weakening of tamper-evidence: a tampered body matches neither
    /// derivation, and the two schemes are domain-separated, so a `v1` id cannot alias
    /// a `v2` one. Nothing publishes under this scheme — [`Self::id`] is the only
    /// write-side identity.
    #[must_use]
    pub fn id_v1(&self) -> ManifestId {
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
