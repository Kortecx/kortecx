// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! [`TaskSignature`] (M7.0, D82) — the verdict-reuse identity foundation: the
//! closed [`SignatureAxis`] vocabulary, the content-addressed
//! [`TaskSignatureHash`], and the [`VerdictScope`] reuse association.
//!
//! A `TaskSignature` is built ONLY via [`TaskSignature::model_invariant`] or
//! [`TaskSignature::scoped`]; its fields are private, so a malformed signature
//! (e.g. a caller-forged `schema_version`, or an axis outside the closed set) is
//! **unrepresentable**. Identity is `blake3` over a domain-tagged canonical
//! bincode encoding — the same discipline as `kx_workflow::ManifestId` and
//! `kx_mote::MoteDef::hash`, so two byte-identical signatures share a hash and a
//! shared signature is verifiable by reference.

use std::collections::BTreeSet;

use kx_mote::{MoteDefHash, MoteId};
use serde::{Deserialize, Serialize};

/// Schema version of the canonical [`TaskSignature`] encoding. Bumped on ANY
/// change to the canonical bytes (a `TaskSignature` folds into a
/// [`TaskSignatureHash`] that scopes verdict reuse). Stored as a struct field
/// (mirroring `kx_mote::MOTE_DEF_SCHEMA_VERSION` on `MoteDef`) so it is bound
/// into the hash, and pinned in the hash's domain tag.
pub const TASK_SIGNATURE_SCHEMA_VERSION: u16 = 1;

/// A 32-byte BLAKE3 hash — the common substrate of the catalog's identity types.
type Hash32 = [u8; 32];

/// The 32-byte content-addressed identity of a [`TaskSignature`].
///
/// `blake3(b"kx-catalog/task-signature/v1" ‖ canonical_bincode(signature))`.
/// The domain tag prevents cross-type preimage aliasing (a `TaskSignatureHash`
/// can never collide with a `ManifestId` / `MoteDefHash` preimage). The newtype
/// mirrors `kx_mote::MoteDefHash` / `kx_workflow::ManifestId`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskSignatureHash(pub Hash32);

impl TaskSignatureHash {
    /// Construct from raw 32 bytes.
    #[inline]
    #[must_use]
    pub const fn from_bytes(bytes: Hash32) -> Self {
        Self(bytes)
    }

    /// Borrow the underlying 32 bytes.
    #[inline]
    #[must_use]
    pub const fn as_bytes(&self) -> &Hash32 {
        &self.0
    }

    /// Lowercase 64-char hex.
    #[must_use]
    pub fn to_hex(self) -> String {
        blake3::Hash::from_bytes(self.0).to_hex().to_string()
    }
}

impl std::fmt::Debug for TaskSignatureHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TaskSignatureHash({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl std::fmt::Display for TaskSignatureHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The closed set of `MoteDef` axes a [`TaskSignature::scoped`] signature may
/// narrow on — EXACTLY the five behavior-determining `MoteDef` identity fields.
///
/// Illegal states unrepresentable: a caller cannot name an axis that is not a
/// pinned `MoteDef` input. Growing this enum is a [`TASK_SIGNATURE_SCHEMA_VERSION`]
/// bump (the variant set folds into the canonical bytes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum SignatureAxis {
    /// The citer Mote's `model_id`.
    CiterModelId,
    /// The citer Mote's `prompt_template_hash`.
    CiterPromptTemplateHash,
    /// An entry of the citer Mote's `tool_contract`.
    CiterToolContractEntry,
    /// The citer Mote's `inference_params`.
    CiterInferenceParams,
    /// A key of the citer Mote's `config_subset`.
    CiterConfigKey,
}

/// The reuse scope of a deterministic critic's `Valid` verdict: "this critic's
/// `Valid` verdict is reusable for runs whose task matches this signature".
///
/// Recorded in the CATALOG ([`crate::SignatureEntry`]), **never** on `MoteDef` —
/// keeping `kx-mote` byte-unchanged and the canonical digest invariant. The
/// promotion gate itself stays the exact, fail-closed
/// `kx_projection::promotion` check; a `VerdictScope` is reuse bookkeeping, not
/// a gate bypass (SN-8).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct VerdictScope {
    /// The critic-bearing Mote whose `Valid` verdict the scope makes reusable.
    pub citee_mote_id: MoteId,
    /// The signature the reuse is scoped to (raw bytes of a [`TaskSignatureHash`]).
    pub task_signature_hash: Hash32,
}

/// The verdict-reuse identity foundation (M7.0, D82).
///
/// Built ONLY via [`TaskSignature::model_invariant`] (empty narrowing) or
/// [`TaskSignature::scoped`]. `narrowing` is the set of `MoteDef` axes whose
/// value must additionally match for reuse; an empty set IS model-invariance.
/// The terminating deterministic critic is pinned by its `MoteDefHash`. All
/// fields are private — the constructors are the only construction path, so
/// `schema_version` is never caller-set.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct TaskSignature {
    /// Canonical-encoding schema version (constructor-set, never caller-set).
    schema_version: u16,
    /// The `MoteDefHash` of the deterministic critic terminating the chain.
    critic_mote_def_hash: MoteDefHash,
    /// The closed, canonical (`BTreeSet`-ordered) set of narrowed axes. Empty
    /// <=> model-invariant.
    narrowing: BTreeSet<SignatureAxis>,
}

impl TaskSignature {
    /// A model-invariant signature: empty narrowing. Reuse matches any citer
    /// differing only in axes NOT pinned here (i.e. all of them).
    #[must_use]
    pub fn model_invariant(critic_mote_def_hash: MoteDefHash) -> Self {
        Self {
            schema_version: TASK_SIGNATURE_SCHEMA_VERSION,
            critic_mote_def_hash,
            narrowing: BTreeSet::new(),
        }
    }

    /// A scoped signature: reuse additionally requires every named axis to
    /// match. Pure + total + infallible — a `BTreeSet` is canonical by
    /// construction and every [`SignatureAxis`] is a legal value, so there is no
    /// malformed-narrowing state to reject. `scoped(h, <empty>)` is byte-identical
    /// to `model_invariant(h)` (an empty narrowing IS model-invariance).
    #[must_use]
    pub fn scoped(critic_mote_def_hash: MoteDefHash, narrowing: BTreeSet<SignatureAxis>) -> Self {
        Self {
            schema_version: TASK_SIGNATURE_SCHEMA_VERSION,
            critic_mote_def_hash,
            narrowing,
        }
    }

    /// The canonical-encoding schema version this signature was built under.
    #[inline]
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// The pinned terminating-critic `MoteDefHash`.
    #[inline]
    #[must_use]
    pub const fn critic_mote_def_hash(&self) -> &MoteDefHash {
        &self.critic_mote_def_hash
    }

    /// The narrowed axes, in canonical (`Ord`) order.
    #[inline]
    #[must_use]
    pub const fn narrowing(&self) -> &BTreeSet<SignatureAxis> {
        &self.narrowing
    }

    /// `true` iff this is a model-invariant signature (empty narrowing).
    #[must_use]
    pub fn is_model_invariant(&self) -> bool {
        self.narrowing.is_empty()
    }

    /// The content-addressed identity: `blake3(domain-tag ‖
    /// canonical_bincode(self))`. A **pure** function of the signature's bytes;
    /// the `schema_version` field is encoded in the body, so a version change
    /// re-derives the hash. The narrowing is a `BTreeSet`, so insertion order
    /// does not affect the hash.
    #[must_use]
    pub fn task_signature_hash(&self) -> TaskSignatureHash {
        let mut h = blake3::Hasher::new();
        h.update(b"kx-catalog/task-signature/v1");
        // SAFETY (expect): TaskSignature has no floats and no non-encodable
        // variants (MoteDefHash = [u8;32], BTreeSet<u8-discriminant enum>, u16),
        // so canonical bincode encoding is infallible — mirrors the documented
        // `kx_mote::MoteDef::hash` / `kx_critic_types::CriticVerdict::encode`.
        let body = bincode::serde::encode_to_vec(self, canonical_config()).expect(
            "TaskSignature canonical encoding is infallible (no floats, no non-encodable types)",
        );
        h.update(&body);
        TaskSignatureHash(*h.finalize().as_bytes())
    }
}

/// The canonical bincode configuration for catalog content-addressing: bincode
/// v2, little-endian, fixed-int. **Byte-identical to `kx_mote::canonical_config`**
/// — replicated here so this crate stays `kx-mote`-internals-free (mirrors
/// `kx_critic_types::canonical_config`). Any change to these flags is a
/// [`TASK_SIGNATURE_SCHEMA_VERSION`] bump.
#[must_use]
pub fn canonical_config(
) -> bincode::config::Configuration<bincode::config::LittleEndian, bincode::config::Fixint> {
    bincode::config::standard()
        .with_little_endian()
        .with_fixed_int_encoding()
}
