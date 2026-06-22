//! [`CheckSpec`] — a deterministic check declared as DATA. A critic Mote carries
//! its `CheckSpec` so the check folds into the Mote's identity (reproducible by
//! construction) and the runtime evaluates it in-process — no opaque binary.
//!
//! Illegal states unrepresentable: a [`CheckSpec`] is exactly one of the four
//! kinds; every set field is a `BTreeSet` (canonical iteration order) and every
//! numeric field is an integer (no floats on the identity path).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::verdict::{CheckKind, PiiClass, StatKind};

/// One check the runtime evaluates against a producer's committed output bytes.
///
/// The first four kinds are **deterministic** in-process checks (see
/// `kx_critic::evaluate`) committed `Pure`. [`CheckSpec::LlmJudge`] is the
/// opt-in **model-graded** kind (T-AGENT2): a `ReadOnlyNondet` gate dispatched
/// by the live serve executor (NOT in-process — `kx_critic::evaluate` fails it
/// closed to `Invalid`). It is a **trailing variant**: the four deterministic
/// variants keep their canonical discriminants, so a `critic_check: None` (the
/// canonical demo) and every existing native critic fold byte-identically —
/// the projection digest is unaffected (no `CRITIC_SCHEMA_VERSION` bump).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckSpec {
    /// Validate that the output conforms to a declared [`SchemaTag`].
    Schema(SchemaSpec),
    /// Reject duplicate records under a declared framing + key.
    Dedup(DedupSpec),
    /// Reject when a declared aggregate leaves an inclusive integer bound.
    StatBounds(StatBoundsSpec),
    /// Reject when a forbidden PII pattern class matches.
    PiiLeak(PiiSpec),
    /// Opt-in **LLM-judge** gate (T-AGENT2): grade the producer output against a
    /// content-addressed rubric using the served model. A `ReadOnlyNondet` gate
    /// — sampled once, committed, replayed (never re-queried). SN-8: the judge
    /// returns a discrete Valid/Invalid decision, **never a similarity score**.
    LlmJudge(LlmJudgeSpec),
}

/// The element type of a tensor payload. Self-contained mirror of
/// `kx_dataset::TensorDType` (this crate is deliberately `kx-dataset`-free; see
/// the crate docs). Stable `as_u8` tag for canonical folding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TensorDTypeTag {
    /// 32-bit IEEE float.
    F32,
    /// 16-bit IEEE float.
    F16,
    /// bfloat16.
    BF16,
    /// 64-bit signed integer.
    I64,
    /// 32-bit signed integer.
    I32,
    /// unsigned byte.
    U8,
    /// boolean (one byte per element).
    Bool,
}

impl TensorDTypeTag {
    /// Stable u8 tag for content-addressed folding. MUST NOT change without a
    /// `CRITIC_SCHEMA_VERSION` bump. Matches `kx_dataset::TensorDType::as_u8`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            TensorDTypeTag::F32 => 0,
            TensorDTypeTag::F16 => 1,
            TensorDTypeTag::BF16 => 2,
            TensorDTypeTag::I64 => 3,
            TensorDTypeTag::I32 => 4,
            TensorDTypeTag::U8 => 5,
            TensorDTypeTag::Bool => 6,
        }
    }

    /// The on-disk byte width of one element of this dtype. Used by the schema
    /// check to validate a tensor/vector payload's total length.
    #[must_use]
    pub const fn byte_width(self) -> u64 {
        match self {
            TensorDTypeTag::F32 | TensorDTypeTag::I32 => 4,
            TensorDTypeTag::F16 | TensorDTypeTag::BF16 => 2,
            TensorDTypeTag::I64 => 8,
            TensorDTypeTag::U8 | TensorDTypeTag::Bool => 1,
        }
    }
}

/// The declared type a schema check validates against. Self-contained mirror of
/// `kx_dataset::ContentSchema` (`kx-critic` offers a `From<&ContentSchema>`
/// conversion). The stable per-variant u8 tags match `ContentSchema::hash_into`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaTag {
    /// Untyped bytes — always conforms.
    Blob,
    /// UTF-8 text.
    Text,
    /// A well-formed JSON document.
    Json,
    /// A dense tensor of `dtype` with the given row-major `shape`.
    Tensor {
        /// Element type.
        dtype: TensorDTypeTag,
        /// Dimensions, row-major. Empty = scalar.
        shape: SmallVec<[u64; 4]>,
    },
    /// An embedding vector of `dim` `f32`s.
    Vector {
        /// Dimensionality.
        dim: u32,
    },
    /// An image payload (forward stub; no structural constraint yet).
    Image,
    /// An audio payload (forward stub; no structural constraint yet).
    Audio,
}

impl SchemaTag {
    /// Fold the schema tag into a hasher canonically (stable tag + parameters).
    /// Byte-identical to `kx_dataset::ContentSchema::hash_into`.
    pub(crate) fn hash_into(&self, h: &mut blake3::Hasher) {
        match self {
            SchemaTag::Blob => {
                h.update(&[0]);
            }
            SchemaTag::Text => {
                h.update(&[1]);
            }
            SchemaTag::Json => {
                h.update(&[2]);
            }
            SchemaTag::Tensor { dtype, shape } => {
                h.update(&[3, dtype.as_u8()]);
                h.update(&(shape.len() as u64).to_le_bytes());
                for dim in shape {
                    h.update(&dim.to_le_bytes());
                }
            }
            SchemaTag::Vector { dim } => {
                h.update(&[4]);
                h.update(&dim.to_le_bytes());
            }
            SchemaTag::Image => {
                h.update(&[5]);
            }
            SchemaTag::Audio => {
                h.update(&[6]);
            }
        }
    }
}

/// Spec for the schema check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaSpec {
    /// The schema the output bytes must satisfy.
    pub expected: SchemaTag,
}

/// How records are framed within the producer's output bytes. Closed enum so the
/// dedup / stat parsers are total and declarative (no opaque callback).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordFraming {
    /// One record per LF-delimited line (a trailing newline is optional and does
    /// NOT produce a final empty record).
    LinesLf,
    /// Length-prefixed records: `u32`-LE byte length followed by that many bytes,
    /// repeated to end of input.
    LengthPrefixedU32,
    /// Fixed-width records of exactly `width` bytes each.
    FixedWidth {
        /// Record width in bytes (must be non-zero; a zero width is treated as
        /// an `Unparseable` framing error at evaluation, never a panic).
        width: u32,
    },
}

impl RecordFraming {
    fn hash_into(self, h: &mut blake3::Hasher) {
        match self {
            RecordFraming::LinesLf => h.update(&[0]),
            RecordFraming::LengthPrefixedU32 => h.update(&[1]),
            RecordFraming::FixedWidth { width } => {
                h.update(&[2]);
                h.update(&width.to_le_bytes())
            }
        };
    }
}

/// Spec for the dedup check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DedupSpec {
    /// How to split the output into records.
    pub framing: RecordFraming,
    /// Dedup on the whole record (`None`) or on a `[start, end)` byte sub-range
    /// of each record (`Some`). An out-of-range key range yields `Unparseable`.
    pub key_range: Option<(u32, u32)>,
}

/// Spec for the stat-bounds check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatBoundsSpec {
    /// How to split the output into records.
    pub framing: RecordFraming,
    /// Which statistic to bound.
    pub stat: StatKind,
    /// Fixed-point scale of the numeric field + bounds (e.g. `1000` = three
    /// decimal places). Numeric fields are parsed as scaled integers; means use
    /// integer division (documented truncation toward zero).
    pub scale: u32,
    /// Inclusive lower bound, in scaled-integer space.
    pub lo_scaled: i64,
    /// Inclusive upper bound, in scaled-integer space.
    pub hi_scaled: i64,
    /// For `MeanScaled` / `MinScaled` / `MaxScaled`: the `[start, end)` byte
    /// sub-range of each record holding the scaled-integer numeric field
    /// (ASCII decimal). Ignored for `RecordCount`. `None` parses the whole
    /// record as the field.
    pub numeric_field_range: Option<(u32, u32)>,
}

/// Spec for the PII-leakage check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PiiSpec {
    /// The detector classes to forbid. `BTreeSet` gives canonical iteration
    /// order, which is the deterministic priority when several classes match.
    pub forbidden: BTreeSet<PiiClass>,
}

/// Spec for the opt-in LLM-judge gate (T-AGENT2).
///
/// The judge dispatches the **served model** (not a spec-named model — for the
/// OSS single-served-model RC the served model IS the judge; a future PR can add
/// a per-spec model selector) over a rubric + the producer's committed bytes,
/// and parses the completion into a discrete `Valid`/`Invalid` verdict.
///
/// The rubric (the grading instruction) is delivered via the judge Mote's
/// `config_subset[JUDGE_RUBRIC_KEY]` (identity-bearing, like a model step's
/// prompt) — NOT carried here — so two judges with different rubrics are distinct
/// Motes by construction without a content-store read on the hot path. This spec
/// carries only the integer output-token bound (the verdict is a short discrete
/// decision). Integer-only — **no floats** on the identity path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmJudgeSpec {
    /// Authored fail-closed cap on the judge model's output tokens. Folds into the
    /// critic Mote's `MoteId`; the recipe also mirrors it into the step warrant's
    /// `model_route.max_output_tokens`, which `inference_params_from_mote` enforces.
    pub max_output_tokens: u32,
}

impl CheckSpec {
    /// Which check kind this is.
    #[must_use]
    pub const fn kind(&self) -> CheckKind {
        match self {
            CheckSpec::Schema(_) => CheckKind::Schema,
            CheckSpec::Dedup(_) => CheckKind::Dedup,
            CheckSpec::StatBounds(_) => CheckKind::StatBounds,
            CheckSpec::PiiLeak(_) => CheckKind::PiiLeak,
            CheckSpec::LlmJudge(_) => CheckKind::LlmJudge,
        }
    }

    /// `true` iff this is the opt-in model-graded [`CheckSpec::LlmJudge`] gate
    /// (T-AGENT2) — a `ReadOnlyNondet` critic dispatched by the live serve
    /// executor — as opposed to one of the four deterministic, `Pure`,
    /// in-process checks. An inherent helper so downstream crates that hold a
    /// `CheckSpec` (the refusal gate, the worker dispatch) can branch the judge
    /// vs native path WITHOUT importing the type or matching its variants.
    #[must_use]
    pub const fn is_llm_judge(&self) -> bool {
        matches!(self, CheckSpec::LlmJudge(_))
    }

    /// Fold this spec into a hasher canonically. Stable per-variant u8 tag,
    /// little-endian integers, `BTreeSet` iterated in `Ord` order. Two equal
    /// specs fold identically; two differing specs fold differently (the
    /// identity-discrimination guarantee that makes a critic's check part of its
    /// `MoteId`).
    ///
    /// This is the secondary / test surface. PR-2's `MoteDef::hash` carries the
    /// spec via canonical bincode over the embedded `CheckSpec`; this method
    /// freezes the same byte intent independently.
    pub fn hash_into(&self, h: &mut blake3::Hasher) {
        match self {
            CheckSpec::Schema(s) => {
                h.update(&[0]);
                s.expected.hash_into(h);
            }
            CheckSpec::Dedup(s) => {
                h.update(&[1]);
                s.framing.hash_into(h);
                hash_key_range(h, s.key_range);
            }
            CheckSpec::StatBounds(s) => {
                h.update(&[2]);
                s.framing.hash_into(h);
                h.update(&[stat_tag(s.stat)]);
                h.update(&s.scale.to_le_bytes());
                h.update(&s.lo_scaled.to_le_bytes());
                h.update(&s.hi_scaled.to_le_bytes());
                hash_key_range(h, s.numeric_field_range);
            }
            CheckSpec::PiiLeak(s) => {
                h.update(&[3]);
                h.update(&(s.forbidden.len() as u64).to_le_bytes());
                for class in &s.forbidden {
                    h.update(&[pii_tag(*class)]);
                }
            }
            CheckSpec::LlmJudge(s) => {
                h.update(&[4]);
                h.update(&s.max_output_tokens.to_le_bytes());
            }
        }
    }
}

fn hash_key_range(h: &mut blake3::Hasher, range: Option<(u32, u32)>) {
    match range {
        None => h.update(&[0]),
        Some((start, end)) => {
            h.update(&[1]);
            h.update(&start.to_le_bytes());
            h.update(&end.to_le_bytes())
        }
    };
}

const fn stat_tag(stat: StatKind) -> u8 {
    match stat {
        StatKind::RecordCount => 0,
        StatKind::MeanScaled => 1,
        StatKind::MinScaled => 2,
        StatKind::MaxScaled => 3,
    }
}

const fn pii_tag(class: PiiClass) -> u8 {
    match class {
        PiiClass::Email => 0,
        PiiClass::IpV4 => 1,
        PiiClass::CreditCardLuhn => 2,
        PiiClass::UsSsn => 3,
    }
}
