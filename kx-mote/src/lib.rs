#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::unnested_or_patterns,
    clippy::redundant_closure_for_method_calls
)]

//! # kx-mote — the atomic execution unit
//!
//! This crate defines the *Mote*: the smallest indivisible thing the kortecx runtime
//! schedules, recovers, and reasons about. Per the design corpus (private), a Mote is
//! a durably-recorded record-of-an-attempt-to-effect-something — the deliberate
//! inversion of Spark's recomputable RDD. A committed Mote is a fact in the journal
//! and is never re-run; recovery is replay-from-journal.
//!
//! The crate is a *pure-types* crate. It has **no I/O, no async, and no runtime
//! dependencies** — only `blake3`, `serde`, `bincode`, `smallvec`, and `thiserror`.
//! Every downstream kortecx crate (journal, projection, executor, scheduler, runtime)
//! imports types from here; it is the narrow waist.
//!
//! ## What lives here
//!
//! - The [`MoteId`] 32-byte identity type and its derivation from `MoteDef`-hash +
//!   `input_data_id` + `graph_position`.
//! - The [`MoteDef`] struct and its canonical [`MoteDef::hash`] over a frozen bincode
//!   configuration ([`canonical_config`]).
//! - The [`Mote`] runtime-instance type that composes a `MoteDef` with per-instance
//!   position data and parent edges.
//! - The non-determinism tag [`NdClass`] (PURE / READ-ONLY-NONDET / WORLD-MUTATING).
//! - The [`EffectPattern`] enum declaring which of the three effect/commit patterns
//!   a Mote uses (idempotent-by-construction / stage-then-commit / validate-then-commit).
//! - Typed dependency edges via [`EdgeKind`] and [`EdgeMeta`].
//! - The per-attempt lifecycle state machine ([`AttemptState`], [`transition`]).
//! - A minimal [`MoteGraph`] container for workflow-author-side composition.
//!
//! ## What does NOT live here
//!
//! - Journal types, content-store types, projection logic, executor logic, inference,
//!   networking — those land in their respective `kx-*` crates.
//! - Runtime behavior. This crate only defines the *shapes* the runtime moves around.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Hash newtypes
// ---------------------------------------------------------------------------

/// A 32-byte BLAKE3 hash. Common substrate for all hash newtypes in the crate.
type Hash32 = [u8; 32];

/// The stable 32-byte identity of a Mote (see crate-level docs).
///
/// Derived purely from the workflow definition, the committed inputs the Mote
/// consumes, and its position in the DAG — never from clock, host, PID, or
/// attempt number. Two workers attempting the same logical work derive the
/// same `MoteId`; the journal dedupes them to one committed fact.
///
/// # Examples
///
/// ```
/// use kx_mote::MoteId;
///
/// let a = MoteId::from_bytes([0xaa; 32]);
/// let b = MoteId::from_bytes([0xaa; 32]);
/// assert_eq!(a, b, "MoteId equality is by-bytes");
/// assert_eq!(a.as_bytes(), &[0xaa; 32]);
///
/// // Display + Debug both render the 64-char lowercase hex form.
/// let hex = format!("{}", a);
/// assert_eq!(hex.len(), 64);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MoteId(pub Hash32);

impl MoteId {
    /// Construct a `MoteId` from raw 32 bytes.
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
}

impl std::fmt::Debug for MoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MoteId({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

impl std::fmt::Display for MoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte canonical hash of a [`MoteDef`].
///
/// Computed by [`MoteDef::hash`]: serialize `MoteDef` with [`canonical_config`]
/// (a frozen bincode configuration), then BLAKE3 the resulting bytes. Identifies
/// a Mote's *kind of work* in the journal; the poison-cascade (definition-level
/// repudiation) queries committed entries by this hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MoteDefHash(pub Hash32);

impl MoteDefHash {
    /// Construct a `MoteDefHash` from raw 32 bytes.
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
}

impl std::fmt::Debug for MoteDefHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MoteDefHash({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl std::fmt::Display for MoteDefHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte identity of the actual upstream inputs a Mote consumes.
///
/// Derived from the `result_ref` content hashes of the Mote's committed parents
/// (executor-owned derivation in P1.9). For zero-parent (entrypoint) Motes, it
/// is the BLAKE3 of a per-run workflow-input seed; this crate stores the
/// pre-computed bytes and never invents the derivation.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InputDataId(pub Hash32);

impl InputDataId {
    /// Construct an `InputDataId` from raw 32 bytes.
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
}

impl std::fmt::Debug for InputDataId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "InputDataId({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

/// The 32-byte hash of the compiled artifact backing a Mote's logic (its `logic_ref`).
///
/// The reproducible-build discipline (workspace-level, P1.1) ensures this hash is
/// stable across machines and CI runs. Component of [`MoteDef::logic_ref`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogicRef(pub Hash32);

impl LogicRef {
    /// Construct a `LogicRef` from raw 32 bytes.
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
}

impl std::fmt::Debug for LogicRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LogicRef({})", blake3::Hash::from_bytes(self.0).to_hex())
    }
}

/// The 32-byte hash of a Mote's system/prompt template.
///
/// A change in prompt template materially changes what the Mote commits; this
/// hash is part of [`MoteDef`] so the change flows through `mote_def_hash`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PromptTemplateHash(pub Hash32);

impl PromptTemplateHash {
    /// Construct a `PromptTemplateHash` from raw 32 bytes.
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
}

impl std::fmt::Debug for PromptTemplateHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PromptTemplateHash({})",
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

// ---------------------------------------------------------------------------
// Identifier newtypes (strings / bytes)
// ---------------------------------------------------------------------------

/// Pinned identity of an inference model, *inclusive of version and quantization*.
///
/// Workflow authors are responsible for packing version and quantization into
/// this identifier — two models with the same name but different quantizations
/// MUST produce different `ModelId`s, or behavior drifts silently across runs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModelId(pub String);

/// The name of a tool a Mote may call. Paired with [`ToolVersion`] in
/// [`MoteDef::tool_contract`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolName(pub String);

/// The version of a tool a Mote may call. A version bump materially changes
/// what a Mote commits and so changes its identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ToolVersion(pub String);

/// A key in the curated `config_subset` allowlist of [`MoteDef`].
///
/// **Discipline (closes I2 from `02-improvement-areas.md`).** Only
/// *behavior-affecting* keys belong here. Log-level, telemetry endpoints,
/// worker thread count, and other operational knobs MUST be excluded —
/// including them would re-fire the identity hash on operational tweaks
/// without any change to what the Mote commits. Maintaining this allowlist
/// is a deliberate, reviewed act; the workflow SDK (P4) will surface the
/// review point.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConfigKey(pub String);

/// The byte-encoded value of a `config_subset` entry. Opaque to this crate;
/// the workflow author decides the encoding (typically a serialized scalar
/// or small struct). Bincode canonical-serializes the bytes verbatim.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConfigVal(pub Vec<u8>);

/// The stable position of a Mote in its DAG.
///
/// Assigned at DAG-compile time (workflow SDK) or derived from a topology
/// shaper's `TopologyDecision` for shaper-spawned children (per `topology.md`
/// §7 / D23: child positions extend the shaper's by appending the child's
/// u32 index in `TopologyDecision.children`). Opaque bytes to this crate;
/// participates in [`MoteId`] derivation alongside `mote_def_hash` and
/// `input_data_id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GraphPosition(pub Vec<u8>);

// ---------------------------------------------------------------------------
// NdClass — the non-determinism tag (one knob, three jobs)
// ---------------------------------------------------------------------------

/// The non-determinism tag attached to every Mote.
///
/// One knob, three jobs (recovery, storage tiering, scheduling priority).
/// See the private design corpus (`mote.md` §6 + D2) for the per-tag rules.
/// Stable u8 representations are used in journal entry headers (PURE=0,
/// READ-ONLY-NONDET=1, WORLD-MUTATING=2) — these MUST NOT change without
/// a journal `schema_version` bump.
///
/// # Examples
///
/// ```
/// use kx_mote::NdClass;
///
/// // Stable u8 discriminants for journal-entry headers.
/// assert_eq!(NdClass::Pure.as_u8(), 0);
/// assert_eq!(NdClass::ReadOnlyNondet.as_u8(), 1);
/// assert_eq!(NdClass::WorldMutating.as_u8(), 2);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum NdClass {
    /// Output is a mathematically-deterministic AND bit-stable function of inputs.
    /// No side effects, no model sampling, no external calls. Safe to re-run.
    /// Storage: droppable + recomputable under memory pressure.
    Pure = 0,

    /// Samples a non-deterministic source (model inference, RNG) but causes
    /// **no external state change**. NEVER re-run once Committed; recovery
    /// reads the committed result. Storage: always persisted.
    ReadOnlyNondet = 1,

    /// Causes external side effects (API call, write, message send) the runtime
    /// cannot reverse or recompute. NEVER re-run once Committed; pre-commit
    /// re-runs are safe only via [`EffectPattern::IdempotentByConstruction`] or
    /// [`EffectPattern::ValidateThenCommit`]. Storage: always persisted.
    /// Speculation forbidden by the executor.
    WorldMutating = 2,
}

impl NdClass {
    /// Convert to the canonical u8 representation for journal entry headers.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

// ---------------------------------------------------------------------------
// EffectPattern — which effect/commit pattern this Mote uses
// ---------------------------------------------------------------------------

/// Declares which of the three effect/commit patterns a Mote uses
/// (`mote.md` §4, D20).
///
/// Read by the executor's submission-time refusal predicate
/// (`validate-then-commit.md` §7) to enforce the safety contract: a
/// WORLD-MUTATING Mote without an idempotency mechanism AND without a critic
/// is refused at submission. The field is REQUIRED (not `Option`); workflow
/// authors must declare a pattern explicitly.
///
/// # Examples
///
/// ```
/// use kx_mote::EffectPattern;
///
/// // The three patterns are mutually exclusive; a Mote picks exactly one.
/// let payment = EffectPattern::IdempotentByConstruction; // Stripe-style
/// let llm_output = EffectPattern::StageThenCommit;       // payload IS the effect
/// let critical_write = EffectPattern::ValidateThenCommit;// gated by a critic
///
/// assert_ne!(payment, llm_output);
/// assert_ne!(llm_output, critical_write);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EffectPattern {
    /// The effect carries an idempotency mechanism the external system honors
    /// (Stripe-style idempotency-key header, content-derived URL, deterministic
    /// resource path, unique-constraint INSERT). Safe to retry.
    IdempotentByConstruction,

    /// The effect produces a payload; the executor stages the payload into the
    /// content store and atomically commits the `result_ref`. Crashes before
    /// the txn lands leave the staged payload orphaned (GC-able). Most natural
    /// for pure-output WORLD-MUTATING work where the effect IS the payload.
    StageThenCommit,

    /// The effect proposes (writes to staging or makes a "draft" call); a
    /// downstream critic Mote validates; only on a valid verdict does the
    /// runtime promote to a committed effect. Full mechanics in
    /// `validate-then-commit.md` (D20).
    ValidateThenCommit,
}

// ---------------------------------------------------------------------------
// Edges — typed dependencies between Motes
// ---------------------------------------------------------------------------

/// The type of a directed dependency edge between two Motes (`mote.md` §5, D6).
///
/// Stable u8 representations are used in journal `ParentEntry` encoding
/// (Data=0, Control=1) — these MUST NOT change without a journal
/// `schema_version` bump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum EdgeKind {
    /// A's `ContentRef` is in B's `input_data_id`. Repudiating A always cascades
    /// to B (no opt-out — B read an invalidated input).
    Data = 0,

    /// A must be Committed before B runs, but A's output is not consumed by B.
    /// Cascades by default (the asymmetry rule, D7); per-edge author-declared
    /// causation-only opt-out via [`EdgeMeta::non_cascade`].
    Control = 1,
}

impl EdgeKind {
    /// Convert to the canonical u8 representation used by the journal.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Per-edge metadata attached to a dependency edge (`projection.md` §7;
/// matches `journal-entry.md` §5 `ParentEntry`).
///
/// `non_cascade` is the per-edge author-declared causation-only opt-out, valid
/// **only** when `kind == EdgeKind::Control`. The encoder for journal entries
/// asserts `non_cascade == false` for Data edges (anti-pattern #6 in
/// `journal-entry.md` §11) — this crate provides constructors that uphold the
/// rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeMeta {
    /// The kind of dependency this edge expresses.
    pub kind: EdgeKind,
    /// Whether this edge is exempt from the repudiation cascade. Only valid
    /// when `kind == EdgeKind::Control`. Always `false` for Data edges.
    pub non_cascade: bool,
}

impl EdgeMeta {
    /// Construct a Data edge (always cascades).
    #[inline]
    #[must_use]
    pub const fn data() -> Self {
        Self {
            kind: EdgeKind::Data,
            non_cascade: false,
        }
    }

    /// Construct a Control edge that cascades on repudiation (the default).
    #[inline]
    #[must_use]
    pub const fn control() -> Self {
        Self {
            kind: EdgeKind::Control,
            non_cascade: false,
        }
    }

    /// Construct a Control edge with the per-edge non-cascade opt-out.
    ///
    /// Use this only when the workflow author has explicitly decided that
    /// repudiating the parent should NOT invalidate the child — a real but
    /// exceptional category (`mote.md` §5, D7). This is a reviewed act.
    #[inline]
    #[must_use]
    pub const fn control_non_cascading() -> Self {
        Self {
            kind: EdgeKind::Control,
            non_cascade: true,
        }
    }
}

/// A reference to a parent Mote within a [`Mote`]'s declared dependencies.
///
/// Mirrors the on-disk `ParentEntry` shape (`journal-entry.md` §5, D19) at
/// the type level: a parent's `MoteId` plus its edge metadata. The journal
/// entry's `parents` field is a `SmallVec<[ParentRef; 4]>` in code; the
/// stack-inline storage covers the typical 0–4-parent case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ParentRef {
    /// The parent Mote's identity.
    pub parent_id: MoteId,
    /// The edge connecting this parent to the child.
    pub edge: EdgeMeta,
}

// ---------------------------------------------------------------------------
// MoteDef — the closed set of behavior-determining inputs
// ---------------------------------------------------------------------------

/// The current `MoteDef::schema_version`.
///
/// Bumped to **3** after the P0.6 addition of `is_topology_shaper`
/// (and the prior P0.8 addition of `effect_pattern` + `critic_for` to v2).
/// The schema version is the explicit forward-evolution mechanism: old-shape
/// MoteDefs continue to hash and dedupe normally under their own
/// schema_version; new-shape MoteDefs at v3 incorporate all fields.
pub const MOTE_DEF_SCHEMA_VERSION: u16 = 3;

/// The closed set of behavior-determining inputs that defines a Mote's *kind
/// of work* (`idempotency.md` §"mote_def_hash", D4).
///
/// **The governing principle:** a Mote's definition changes when, and only
/// when, any input that could change what it commits changes. Not source
/// text (over-fires), not author-declared version (under-fires) — this
/// closed set, hashed canonically.
///
/// **Canonical encoding.** Hashed via [`MoteDef::hash`], which serializes
/// the struct with [`canonical_config`] (bincode v2 with little-endian,
/// fixed-int encoding) and BLAKE3s the result. `BTreeMap` fields iterate in
/// key order, giving canonical-by-construction stability across insertion
/// order. The serialization configuration is shared with `JournalEntry`
/// (`journal-entry.md` §2, D19) so a single primitive serves both surfaces.
///
/// **Deviation from `idempotency.md` literal text.** The spec types
/// `tool_contract` as `Vec<(ToolName, ToolVersion)>`; this implementation
/// uses `BTreeMap<ToolName, ToolVersion>` to make canonical iteration order
/// a type-level guarantee. The semantic remains "the set of tools the Mote
/// may call with their versions"; canonical hashing was always the intent.
/// Recorded in the project deviation log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoteDef {
    /// Hash of the compiled artifact backing this Mote's logic. The
    /// reproducible-build discipline (P1.1) ensures stability across machines.
    pub logic_ref: LogicRef,

    /// Pinned model identity, inclusive of version and quantization.
    pub model_id: ModelId,

    /// Hash of the system/prompt template the Mote uses.
    pub prompt_template_hash: PromptTemplateHash,

    /// The closed set of tools the Mote may call, each at its pinned version.
    /// Canonical order is guaranteed by `BTreeMap` iteration.
    pub tool_contract: BTreeMap<ToolName, ToolVersion>,

    /// The non-determinism tag — recovery semantics, storage tier, scheduling
    /// priority all derive from this field.
    pub nd_class: NdClass,

    /// The curated allowlist of behavior-affecting configuration. ONLY keys
    /// whose values change what the Mote commits. Operational keys (logging,
    /// telemetry, thread count) MUST be excluded — see [`ConfigKey`] docs.
    pub config_subset: BTreeMap<ConfigKey, ConfigVal>,

    /// Which of the three effect/commit patterns this Mote uses. Required.
    pub effect_pattern: EffectPattern,

    /// If `Some(producer_mote_id)`, this Mote is a critic for that producer
    /// (the `ValidateThenCommit` 3c pattern, D20). The projection's promotion
    /// query reads this field via `is_critic_of`.
    pub critic_for: Option<MoteId>,

    /// If `true`, this Mote's `result_ref` payload IS a `TopologyDecision`
    /// that the projection materializes children from on commit (D23).
    /// Mutually exclusive with `critic_for == Some(_)` (executor refusal R-8).
    pub is_topology_shaper: bool,

    /// Schema version of the `MoteDef` itself. Bumped on any change to the
    /// struct shape; see [`MOTE_DEF_SCHEMA_VERSION`].
    pub schema_version: u16,
}

impl MoteDef {
    /// Compute the canonical BLAKE3 hash of this `MoteDef`.
    ///
    /// Encoding pipeline (frozen):
    /// 1. Serialize the struct with [`canonical_config`] (bincode v2,
    ///    little-endian, fixed-int).
    /// 2. BLAKE3-hash the resulting bytes → 32-byte [`MoteDefHash`].
    ///
    /// Two `MoteDef`s constructed with `BTreeMap` insertions in different
    /// orders produce byte-identical encodings and so identical hashes.
    /// Any change to a field that affects the encoding bytes (including
    /// `schema_version`) re-derives the hash.
    #[must_use]
    pub fn hash(&self) -> MoteDefHash {
        let bytes = bincode::serde::encode_to_vec(self, canonical_config())
            .expect("MoteDef serialization is infallible (no floats, no non-encodable types)");
        let digest = blake3::hash(&bytes);
        MoteDefHash::from_bytes(*digest.as_bytes())
    }
}

/// The canonical bincode configuration used to serialize types whose bytes
/// participate in identity hashes or journal entries.
///
/// Frozen flags (per `journal-entry.md` §2, D19; matches `idempotency.md`
/// §"Non-negotiable" #2):
/// - Little-endian byte order.
/// - Fixed-int encoding (no varint).
/// - Length-prefixed strings (u64 prefix; follows from fixed-int).
/// - No length limits at the encoder level (size caps enforced upstream).
///
/// Any change to these flags is a `schema_version` bump.
#[must_use]
pub fn canonical_config(
) -> bincode::config::Configuration<bincode::config::LittleEndian, bincode::config::Fixint> {
    bincode::config::standard()
        .with_little_endian()
        .with_fixed_int_encoding()
}

// ---------------------------------------------------------------------------
// MoteId derivation
// ---------------------------------------------------------------------------

/// Derive a [`MoteId`] from its three identity components.
///
/// The formula (`idempotency.md` §"full key"):
///
/// ```text
/// MoteId = blake3(mote_def_hash ‖ input_data_id ‖ graph_position)
/// ```
///
/// `mote_def_hash` and `input_data_id` are fixed 32-byte values, so the
/// concatenation boundary is unambiguous even with a variable-length
/// `graph_position` suffix. The three components are kept separately
/// queryable in the journal (D11) so the poison-cascade (D22) can answer
/// queries like "every Mote sharing this `mote_def_hash` is now suspect"
/// (`list_committed_by_mote_def_hash`, P1.4 R-E).
#[must_use]
pub fn derive_mote_id(
    mote_def_hash: &MoteDefHash,
    input_data_id: &InputDataId,
    graph_position: &GraphPosition,
) -> MoteId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(mote_def_hash.as_bytes());
    hasher.update(input_data_id.as_bytes());
    hasher.update(&graph_position.0);
    MoteId::from_bytes(*hasher.finalize().as_bytes())
}

// ---------------------------------------------------------------------------
// Mote — the runtime-instance shape
// ---------------------------------------------------------------------------

/// A Mote instance: a `MoteDef` paired with the per-instance position data
/// (`input_data_id`, `graph_position`) and declared parents, plus the
/// computed [`MoteId`].
///
/// Construct via [`Mote::new`] to ensure `id` is derived correctly from the
/// other fields. The `id` field is exposed for read access but not for
/// independent mutation; any mutation of `def`, `input_data_id`, or
/// `graph_position` invalidates `id` and must go through reconstruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mote {
    /// The computed identity. Read-only after construction.
    pub id: MoteId,
    /// The behavior-determining inputs.
    pub def: MoteDef,
    /// The committed-input identity (executor-derived; opaque here).
    pub input_data_id: InputDataId,
    /// This Mote's stable position in the DAG.
    pub graph_position: GraphPosition,
    /// Declared parent edges. Up to 4 parents stack-inline; spillover heap-allocates.
    pub parents: SmallVec<[ParentRef; 4]>,
}

impl Mote {
    /// Build a `Mote`, computing the [`MoteId`] from its components.
    ///
    /// The constructor is the single entry point that guarantees `id`
    /// matches the rest of the struct — direct struct literal construction
    /// is supported by public fields, but callers are responsible for
    /// maintaining the invariant if they go that route.
    #[must_use]
    pub fn new(
        def: MoteDef,
        input_data_id: InputDataId,
        graph_position: GraphPosition,
        parents: SmallVec<[ParentRef; 4]>,
    ) -> Self {
        let def_hash = def.hash();
        let id = derive_mote_id(&def_hash, &input_data_id, &graph_position);
        Self {
            id,
            def,
            input_data_id,
            graph_position,
            parents,
        }
    }

    /// The Mote's non-determinism tag (mirrors `def.nd_class` for fast access).
    #[inline]
    #[must_use]
    pub const fn nd_class(&self) -> NdClass {
        self.def.nd_class
    }

    /// The Mote's declared effect pattern.
    #[inline]
    #[must_use]
    pub const fn effect_pattern(&self) -> EffectPattern {
        self.def.effect_pattern
    }
}

// ---------------------------------------------------------------------------
// Lifecycle state machine (attempt-scoped, D3)
// ---------------------------------------------------------------------------

/// The per-attempt lifecycle state of a Mote (`mote.md` §7).
///
/// **Attempt-scoped, not Mote-scoped.** The Mote's *identity* (the [`MoteId`])
/// may have many attempts in the journal — e.g., `Failed`, `Failed`,
/// `Committed`. The journal records all attempts; the projection collapses
/// them to a per-identity current state with the precedence rules in
/// `projection.md` §4. This enum describes ONE attempt.
///
/// Stable u8 representations are not assigned here — `AttemptState` is an
/// in-memory model only. The journal carries the *fact of an attempt's
/// outcome* (`Proposed` / `Committed` / `Failed` / `Repudiated` entries),
/// not the running state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AttemptState {
    /// The Mote exists in the DAG but the scheduler has not selected it.
    Pending,
    /// The scheduler has selected the Mote for placement; a `Proposed`
    /// journal entry has been written.
    Scheduled,
    /// A worker has accepted the Mote and begun execution. **Not durable** —
    /// "currently running" is an intent, tracked in worker memory only.
    Running,
    /// The atomic journal txn writing the `Committed` entry has landed.
    Committed,
    /// The attempt reached a terminal failure (typed error, retries
    /// exhausted, validator rejection). A `Failed` journal entry has been
    /// written. Future attempts under the same identity are independent.
    Failed,
    /// The committed result has been explicitly invalidated (operator action,
    /// critic verdict, upstream cascade per D22). A `Repudiated` journal
    /// entry referencing the original `Committed` has been written. Terminal
    /// for this attempt; the log is append-only.
    Repudiated,
}

/// An illegal-transition error from [`transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("illegal Mote attempt transition: {from:?} → {to:?}")]
pub struct IllegalTransition {
    /// The state the attempt was in.
    pub from: AttemptState,
    /// The state the caller attempted to move to.
    pub to: AttemptState,
}

/// Validate a per-attempt lifecycle transition.
///
/// Returns `Ok(to)` if the transition is one of the five legal transitions
/// (`Pending → Scheduled`, `Scheduled → Running`, `Running → Committed`,
/// `Running → Failed`, `Committed → Repudiated`); otherwise returns
/// [`IllegalTransition`]. Every other from→to pair in the 6×6 state matrix
/// is illegal, including same-state self-loops, `Failed → *`, and
/// `Repudiated → *` (both terminal).
///
/// This function is the single source of truth for the transition rules.
/// `kx-executor` (P1.9) and `kx-coordinator` (P2.2) call it before writing
/// any journal entry that would advance an attempt's state.
///
/// # Examples
///
/// ```
/// use kx_mote::{transition, AttemptState};
///
/// // Legal: Pending → Scheduled
/// assert_eq!(
///     transition(AttemptState::Pending, AttemptState::Scheduled).unwrap(),
///     AttemptState::Scheduled
/// );
///
/// // Illegal: Pending → Running (must go through Scheduled first)
/// assert!(transition(AttemptState::Pending, AttemptState::Running).is_err());
///
/// // Illegal: Committed → Failed (Committed only transitions to Repudiated)
/// assert!(transition(AttemptState::Committed, AttemptState::Failed).is_err());
///
/// // Illegal: same-state self-loop
/// assert!(transition(AttemptState::Running, AttemptState::Running).is_err());
/// ```
pub fn transition(from: AttemptState, to: AttemptState) -> Result<AttemptState, IllegalTransition> {
    use AttemptState::{Committed, Failed, Pending, Repudiated, Running, Scheduled};
    let legal = matches!(
        (from, to),
        (Pending, Scheduled)
            | (Scheduled, Running)
            | (Running, Committed)
            | (Running, Failed)
            | (Committed, Repudiated)
    );
    if legal {
        Ok(to)
    } else {
        Err(IllegalTransition { from, to })
    }
}

/// All six [`AttemptState`] variants, for exhaustive iteration in tests and
/// debug tools. Stable order; changes to this constant signal a schema-level
/// adjustment.
pub const ALL_ATTEMPT_STATES: [AttemptState; 6] = [
    AttemptState::Pending,
    AttemptState::Scheduled,
    AttemptState::Running,
    AttemptState::Committed,
    AttemptState::Failed,
    AttemptState::Repudiated,
];

/// Returns `true` for the five legal per-attempt transitions; `false`
/// otherwise. Pure helper around [`transition`] for code paths that prefer a
/// boolean check (e.g., precondition assertions in tests and debug panes).
#[must_use]
pub fn is_legal_transition(from: AttemptState, to: AttemptState) -> bool {
    transition(from, to).is_ok()
}

// ---------------------------------------------------------------------------
// MoteGraph — workflow-author-side container
// ---------------------------------------------------------------------------

/// A workflow-author-side container of Motes and their declared edges.
///
/// This is a *compile-time* shape — the structure a workflow author (or the
/// P4 SDK) builds before any Mote runs. The runtime never reads this; at
/// execution time the projection (P1.5) folds the journal log into the live
/// graph view. This type exists so workflow code has a typed handle for
/// composition and so unit tests have a convenient builder.
///
/// **Contains no traversal logic** — that lives in `kx-projection`. The
/// shape is plain data: keyed nodes and adjacency lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoteGraph {
    /// All Motes in the graph, keyed by identity.
    pub nodes: BTreeMap<MoteId, Mote>,
    /// Adjacency: for each child, the list of declared parent edges.
    /// Mirrors what lands in each Mote's `parents` field at commit time.
    pub edges: BTreeMap<MoteId, SmallVec<[ParentRef; 4]>>,
}

impl MoteGraph {
    /// Create an empty graph.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a Mote and its declared parents. Overwrites any existing entry
    /// for the same `MoteId` (the caller is responsible for the uniqueness
    /// guarantee at compose time; the journal enforces it at runtime via
    /// dedupe-by-key).
    pub fn insert(&mut self, mote: Mote) {
        let id = mote.id;
        self.edges.insert(id, mote.parents.clone());
        self.nodes.insert(id, mote);
    }

    /// Number of Motes in the graph.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// `true` if the graph contains no Motes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Borrow a Mote by identity.
    #[inline]
    #[must_use]
    pub fn get(&self, id: &MoteId) -> Option<&Mote> {
        self.nodes.get(id)
    }

    /// Borrow the declared parent edges of a Mote.
    #[inline]
    #[must_use]
    pub fn parents_of(&self, id: &MoteId) -> Option<&SmallVec<[ParentRef; 4]>> {
        self.edges.get(id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_def() -> MoteDef {
        MoteDef {
            logic_ref: LogicRef::from_bytes([1u8; 32]),
            model_id: ModelId("test-model:v1:q4".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([2u8; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    #[test]
    fn nd_class_u8_repr_is_stable() {
        assert_eq!(NdClass::Pure.as_u8(), 0);
        assert_eq!(NdClass::ReadOnlyNondet.as_u8(), 1);
        assert_eq!(NdClass::WorldMutating.as_u8(), 2);
    }

    #[test]
    fn edge_kind_u8_repr_is_stable() {
        assert_eq!(EdgeKind::Data.as_u8(), 0);
        assert_eq!(EdgeKind::Control.as_u8(), 1);
    }

    #[test]
    fn edge_meta_constructors_uphold_invariants() {
        assert_eq!(
            EdgeMeta::data(),
            EdgeMeta {
                kind: EdgeKind::Data,
                non_cascade: false
            }
        );
        assert_eq!(
            EdgeMeta::control(),
            EdgeMeta {
                kind: EdgeKind::Control,
                non_cascade: false
            }
        );
        assert_eq!(
            EdgeMeta::control_non_cascading(),
            EdgeMeta {
                kind: EdgeKind::Control,
                non_cascade: true
            }
        );
    }

    #[test]
    fn mote_def_hash_is_deterministic_across_calls() {
        let def = sample_def();
        let h1 = def.hash();
        let h2 = def.hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn schema_version_is_v3() {
        assert_eq!(MOTE_DEF_SCHEMA_VERSION, 3);
        assert_eq!(sample_def().schema_version, 3);
    }

    #[test]
    fn derive_mote_id_is_pure() {
        let def_hash = MoteDefHash::from_bytes([7u8; 32]);
        let input = InputDataId::from_bytes([8u8; 32]);
        let pos = GraphPosition(vec![9, 9, 9]);
        let a = derive_mote_id(&def_hash, &input, &pos);
        let b = derive_mote_id(&def_hash, &input, &pos);
        assert_eq!(a, b);
    }

    #[test]
    fn derive_mote_id_differs_on_any_component_change() {
        let def_hash = MoteDefHash::from_bytes([7u8; 32]);
        let input = InputDataId::from_bytes([8u8; 32]);
        let pos = GraphPosition(vec![9, 9, 9]);
        let base = derive_mote_id(&def_hash, &input, &pos);

        let diff_def = derive_mote_id(&MoteDefHash::from_bytes([6u8; 32]), &input, &pos);
        let diff_input = derive_mote_id(&def_hash, &InputDataId::from_bytes([9u8; 32]), &pos);
        let diff_pos = derive_mote_id(&def_hash, &input, &GraphPosition(vec![1]));

        assert_ne!(base, diff_def);
        assert_ne!(base, diff_input);
        assert_ne!(base, diff_pos);
        assert_ne!(diff_def, diff_input);
    }

    #[test]
    fn mote_id_display_is_64_hex_chars() {
        let id = MoteId::from_bytes([0xab; 32]);
        let s = format!("{id}");
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn legal_transitions_are_accepted() {
        use AttemptState::*;
        assert!(transition(Pending, Scheduled).is_ok());
        assert!(transition(Scheduled, Running).is_ok());
        assert!(transition(Running, Committed).is_ok());
        assert!(transition(Running, Failed).is_ok());
        assert!(transition(Committed, Repudiated).is_ok());
    }

    #[test]
    fn exhaustive_transition_matrix() {
        use AttemptState::*;
        let legal: std::collections::HashSet<(AttemptState, AttemptState)> = [
            (Pending, Scheduled),
            (Scheduled, Running),
            (Running, Committed),
            (Running, Failed),
            (Committed, Repudiated),
        ]
        .into_iter()
        .collect();

        let mut legal_count = 0usize;
        let mut illegal_count = 0usize;
        for &from in &ALL_ATTEMPT_STATES {
            for &to in &ALL_ATTEMPT_STATES {
                let result = transition(from, to);
                if legal.contains(&(from, to)) {
                    assert!(
                        result.is_ok(),
                        "expected legal transition: {from:?} → {to:?}"
                    );
                    legal_count += 1;
                } else {
                    assert!(
                        result.is_err(),
                        "expected illegal transition: {from:?} → {to:?}"
                    );
                    illegal_count += 1;
                }
            }
        }
        assert_eq!(legal_count, 5);
        assert_eq!(illegal_count, 36 - 5);
    }

    #[test]
    fn failed_is_terminal() {
        use AttemptState::*;
        for &to in &ALL_ATTEMPT_STATES {
            assert!(
                transition(Failed, to).is_err(),
                "Failed → {to:?} must be illegal (terminal)"
            );
        }
    }

    #[test]
    fn repudiated_is_terminal() {
        use AttemptState::*;
        for &to in &ALL_ATTEMPT_STATES {
            assert!(
                transition(Repudiated, to).is_err(),
                "Repudiated → {to:?} must be illegal (terminal)"
            );
        }
    }

    #[test]
    fn self_loops_are_illegal() {
        for &s in &ALL_ATTEMPT_STATES {
            assert!(
                transition(s, s).is_err(),
                "{s:?} → {s:?} must be illegal (no self-loops)"
            );
        }
    }

    #[test]
    fn committed_does_not_demote_to_failed() {
        use AttemptState::*;
        assert!(transition(Committed, Failed).is_err());
    }

    #[test]
    fn mote_new_derives_id_correctly() {
        let def = sample_def();
        let input = InputDataId::from_bytes([5u8; 32]);
        let pos = GraphPosition(vec![1, 2, 3]);
        let mote = Mote::new(def.clone(), input, pos.clone(), SmallVec::new());
        let expected = derive_mote_id(&def.hash(), &input, &pos);
        assert_eq!(mote.id, expected);
    }

    #[test]
    fn mote_graph_round_trips_a_single_mote() {
        let def = sample_def();
        let mote = Mote::new(
            def,
            InputDataId::from_bytes([0u8; 32]),
            GraphPosition::default(),
            SmallVec::new(),
        );
        let id = mote.id;
        let mut g = MoteGraph::new();
        g.insert(mote.clone());
        assert_eq!(g.len(), 1);
        assert!(!g.is_empty());
        assert_eq!(g.get(&id), Some(&mote));
        assert_eq!(g.parents_of(&id).map(|v| v.len()), Some(0));
    }

    #[test]
    fn illegal_transition_error_carries_states() {
        use AttemptState::*;
        let err = transition(Pending, Committed).unwrap_err();
        assert_eq!(err.from, Pending);
        assert_eq!(err.to, Committed);
        let s = format!("{err}");
        assert!(s.contains("Pending"));
        assert!(s.contains("Committed"));
    }
}
