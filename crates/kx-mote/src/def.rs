//! [`crate::MoteDef`] — the closed set of behavior-determining inputs that defines
//! a Mote's *kind of work*. Plus [`crate::canonical_config`] (the frozen bincode
//! configuration) and [`crate::derive_mote_id`] (the identity-derivation helper).

use std::collections::BTreeMap;

use kx_critic_types::CheckSpec;
use serde::{Deserialize, Serialize};

use crate::effect::EffectPattern;
use crate::id::{InputDataId, LogicRef, MoteDefHash, MoteId, PromptTemplateHash};
use crate::inference_params::InferenceParams;
use crate::ndclass::NdClass;
use crate::strings::{ConfigKey, ConfigVal, GraphPosition, ModelId, ToolName, ToolVersion};

// ---------------------------------------------------------------------------
// MoteDef — the closed set of behavior-determining inputs
// ---------------------------------------------------------------------------

/// The current `MoteDef::schema_version`.
///
/// Bumped to **5** at P4.2-2 to add the `critic_check` field — a critic Mote's
/// declarative [`CheckSpec`] now participates in `mote_def_hash` so a critic's
/// check is part of its identity (changing the declared check changes the
/// `MoteId`; reproducible-by-construction). The embedded `CheckSpec` is carried
/// via the same canonical bincode used for every other field — and it is
/// **integer-only by construction** (`kx_critic_types` enforces no float on the
/// identity path), so folding it preserves the no-float canonical-hash
/// precondition (SN-8). Prior bumps: **4** at D50 (`inference_params` — decoding
/// parameters `temperature_bps`/`top_p_bps`/`top_k`/`seed`/`stop_tokens`/
/// `grammar`/`max_output_tokens`); **3** at the P0.6 addition of
/// `is_topology_shaper`; **2** at the P0.8 addition of `effect_pattern` and
/// `critic_for`. The schema version is the explicit forward-evolution
/// mechanism: old-shape MoteDefs continue to hash and dedupe normally under
/// their own schema_version (their `MoteId` is a stored journal fact, never
/// re-derived on fold); new-shape MoteDefs at v5 incorporate all fields.
pub const MOTE_DEF_SCHEMA_VERSION: u16 = 5;

/// The closed set of behavior-determining inputs that defines a Mote's *kind
/// of work* (`idempotency.md` §"mote_def_hash", D4).
///
/// **The governing principle:** a Mote's definition changes when, and only
/// when, any input that could change what it commits changes. Not source
/// text (over-fires), not author-declared version (under-fires) — this
/// closed set, hashed canonically.
///
/// **Canonical encoding.** Hashed via [`crate::MoteDef::hash`], which serializes
/// the struct with [`crate::canonical_config`] (bincode v2 with little-endian,
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

    /// Decoding parameters — added at D50 so that two MoteDefs differing
    /// only in `temperature_bps`, `seed`, etc. produce different
    /// `mote_def_hash` and different `MoteId` (closes the pre-D50
    /// memoizer-collision latent bug). The dispatcher reads these via
    /// `kx_inference::inference_params_from_mote` — the SOLE permitted
    /// constructor of dispatch-bound `InferenceParams`. Defaults to
    /// greedy (see [`InferenceParams::default`]).
    pub inference_params: InferenceParams,

    /// If `Some(spec)`, this Mote is a **deterministic critic**: the executor
    /// evaluates `spec` against its producer's committed output bytes in-process
    /// (`kx_critic::evaluate`, no `execvp`) and commits the resulting
    /// `CriticVerdict` as this Mote's `result_ref`. Carried in the identity so
    /// the declared check is part of the critic's `MoteId` (changing the check
    /// changes the Mote — reproducible by construction). The spec is
    /// integer-only (no float on the identity path; SN-8). A native-check Mote
    /// MUST be `Pure` with `critic_for = Some(_)` and `!is_topology_shaper`
    /// (executor refusal R-15). `None` for every non-critic Mote.
    pub critic_check: Option<CheckSpec>,

    /// Schema version of the `MoteDef` itself. Bumped on any change to the
    /// struct shape; see [`MOTE_DEF_SCHEMA_VERSION`].
    pub schema_version: u16,
}

impl MoteDef {
    /// Compute the canonical BLAKE3 hash of this `MoteDef`.
    ///
    /// Encoding pipeline (frozen):
    /// 1. Serialize the struct with [`crate::canonical_config`] (bincode v2,
    ///    little-endian, fixed-int).
    /// 2. BLAKE3-hash the resulting bytes → 32-byte [`crate::MoteDefHash`].
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

/// Derive a [`crate::MoteId`] from its three identity components.
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
