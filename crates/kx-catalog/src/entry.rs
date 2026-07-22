// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`SignatureEntry`] (M7.1, D83) — the registry's value type — plus the
//! snapshot/recipe descriptor ([`RecipeSnapshot`]) and the typed free-param
//! contract ([`FreeParamContract`]).
//!
//! A "snapshot" (D83) is a *recipe fingerprint + free-param contract*: invoking
//! it (later milestones) is always a FRESH registered run, never a replayed
//! result. The `variable_slots` machinery lives OUTSIDE [`TaskSignature`] so the
//! author's variable-vs-fixed declaration never bumps
//! [`crate::TASK_SIGNATURE_SCHEMA_VERSION`].

use std::collections::BTreeMap;

use kx_dataset::DatasetId;
use kx_mote::MoteDefHash;
use kx_workflow::ManifestId;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::signature::{TaskSignature, TaskSignatureHash, VerdictScope};

/// Whether a snapshot input is fixed (part of the recipe) or varies per
/// invocation. Default authoring rule (D101.6): external inputs are
/// [`SlotBinding::Variable`], Mote-produced inputs are [`SlotBinding::Constant`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum SlotBinding {
    /// Fixed by the recipe (Mote-produced).
    Constant,
    /// Supplied per invocation (an external free param).
    Variable,
}

/// A typed free-param slot a snapshot declares. The `schema_ref` is the
/// integration hook to the M5.3 MCP `inputSchema` `validate_args`: it is an
/// opaque content-ref of the declared schema bytes here; the gateway / form
/// layer validates submitted args against it (later FE work). No float ever
/// touches this type (`schema_ref` is bytes, never a parsed value).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FreeParamSlot {
    /// Fixed vs varies-per-invocation.
    pub binding: SlotBinding,
    /// Content-ref (`blake3`) of the slot's declared input schema, or `None` for
    /// an untyped/opaque slot.
    pub schema_ref: Option<[u8; 32]>,
}

impl FreeParamSlot {
    /// A variable slot, optionally typed by a schema content-ref.
    #[must_use]
    pub const fn variable(schema_ref: Option<[u8; 32]>) -> Self {
        Self {
            binding: SlotBinding::Variable,
            schema_ref,
        }
    }

    /// A constant slot (fixed by the recipe).
    #[must_use]
    pub const fn constant() -> Self {
        Self {
            binding: SlotBinding::Constant,
            schema_ref: None,
        }
    }
}

/// The typed free-param contract a snapshot publishes. Reuse passes dynamic
/// params (validated against each slot's `schema_ref` at the gateway) → fresh
/// `input_data_id` → fresh run, same recipe. Slots are keyed by name in a
/// `BTreeMap`, so duplicate names are structurally impossible and iteration is
/// canonical.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct FreeParamContract {
    /// The declared slots, keyed by name (canonical order).
    pub slots: BTreeMap<String, FreeParamSlot>,
}

impl FreeParamContract {
    /// An empty contract.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a slot (builder). Re-declaring a name overwrites it (the
    /// `BTreeMap` cannot hold a duplicate key).
    #[must_use]
    pub fn with_slot(mut self, name: impl Into<String>, slot: FreeParamSlot) -> Self {
        self.slots.insert(name.into(), slot);
        self
    }

    /// Number of declared slots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// `true` if no slots are declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// The snapshot / recipe descriptor (D83): a recipe fingerprint + the free-param
/// contract + an optional pinned corpus. Carries NO cached result — a snapshot
/// invocation is always a fresh registered run (`WorldMutating` work re-runs by
/// default).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct RecipeSnapshot {
    /// The recipe fingerprint carried on the `RunRegistered` journal fact
    /// (discovery/dedup hash). Held as raw bytes so this crate stays off the
    /// `kx-journal` dependency; equals the value the submission layer records.
    pub recipe_fingerprint: [u8; 32],
    /// The free-param contract for this snapshot.
    pub free_params: FreeParamContract,
    /// The produced corpus, if the snapshot pins a concrete result alongside the
    /// recipe (mirrors `kx_workflow::Manifest::with_dataset`).
    pub dataset_id: Option<DatasetId>,
}

impl RecipeSnapshot {
    /// A snapshot of a recipe fingerprint with an empty free-param contract and
    /// no pinned corpus.
    #[must_use]
    pub fn new(recipe_fingerprint: [u8; 32]) -> Self {
        Self {
            recipe_fingerprint,
            free_params: FreeParamContract::new(),
            dataset_id: None,
        }
    }

    /// Attach a free-param contract (builder).
    #[must_use]
    pub fn with_free_params(mut self, free_params: FreeParamContract) -> Self {
        self.free_params = free_params;
        self
    }

    /// Pin a produced corpus (builder).
    #[must_use]
    pub fn with_dataset(mut self, dataset_id: DatasetId) -> Self {
        self.dataset_id = Some(dataset_id);
        self
    }
}

/// The registry's value: a fully-described signature entry. Content-addressed by
/// its [`TaskSignature`]'s hash ([`SignatureEntry::hash`]); immutable once
/// registered.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SignatureEntry {
    /// The verdict-reuse identity foundation.
    pub task_signature: TaskSignature,
    /// The skills (MoteDefs) pinned constant for this signature, by hash. A
    /// `SmallVec` avoids per-entry heap allocation for the common small DAG.
    pub pinned_skill_hashes: SmallVec<[MoteDefHash; 8]>,
    /// Which named inputs vary vs are fixed — OUTSIDE [`TaskSignature`] (no
    /// schema bump). Canonical order.
    pub variable_slots: BTreeMap<String, SlotBinding>,
    /// The recipe-as-product reference this signature is published against.
    pub manifest_ref: ManifestId,
    /// The snapshot/recipe descriptor (recipe fingerprint + free-param contract).
    pub snapshot: RecipeSnapshot,
    /// The verdict-reuse association, recorded HERE (off `MoteDef`) so the
    /// canonical digest stays invariant. The promotion gate still decides on the
    /// exact `CriticVerdict::is_valid` check; this is reuse bookkeeping only.
    pub verdict_scope: Option<VerdictScope>,
}

impl SignatureEntry {
    /// A minimal entry: a signature published against a manifest + snapshot, with
    /// no pinned skills, no variable slots, and no verdict scope. Use the
    /// builders to add the rest.
    #[must_use]
    pub fn new(
        task_signature: TaskSignature,
        manifest_ref: ManifestId,
        snapshot: RecipeSnapshot,
    ) -> Self {
        Self {
            task_signature,
            pinned_skill_hashes: SmallVec::new(),
            variable_slots: BTreeMap::new(),
            manifest_ref,
            snapshot,
            verdict_scope: None,
        }
    }

    /// Pin the subgraph's skill hashes (builder).
    #[must_use]
    pub fn with_pinned_skills(
        mut self,
        pinned_skill_hashes: impl IntoIterator<Item = MoteDefHash>,
    ) -> Self {
        self.pinned_skill_hashes = pinned_skill_hashes.into_iter().collect();
        self
    }

    /// Declare which named inputs vary vs are fixed (builder).
    #[must_use]
    pub fn with_variable_slots(
        mut self,
        variable_slots: impl IntoIterator<Item = (String, SlotBinding)>,
    ) -> Self {
        self.variable_slots = variable_slots.into_iter().collect();
        self
    }

    /// Record the verdict-reuse scope (builder).
    #[must_use]
    pub fn with_verdict_scope(mut self, verdict_scope: VerdictScope) -> Self {
        self.verdict_scope = Some(verdict_scope);
        self
    }

    /// The content-addressed registry key — the entry's signature hash.
    #[must_use]
    pub fn hash(&self) -> TaskSignatureHash {
        self.task_signature.task_signature_hash()
    }
}
