//! [`CriticVerdict`] — the content-addressed fact a deterministic critic Mote
//! commits — plus its closed [`CriticReason`] failure vocabulary and the frozen
//! canonical encoding used to content-address it.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::spec::SchemaTag;

/// Schema version of the [`CriticVerdict`] / `CheckSpec` wire encodings. Bumped
/// on ANY change to the canonical bytes of either (the verdict bytes are a
/// committed content-addressed fact; the spec bytes fold into a critic Mote's
/// `MoteId`). Encoded as the first two bytes of [`CriticVerdict::encode`].
pub const CRITIC_SCHEMA_VERSION: u16 = 1;

/// A critic's committed verdict — the value a critic Mote's `result_ref` payload
/// decodes to.
///
/// **SN-8:** produced by EXACT deterministic evaluation; compared downstream by
/// byte-equality only (the projection's promotion gate reads `Valid` vs
/// `Invalid`, never a score). Two runs over identical producer output produce a
/// byte-identical verdict and therefore an identical content ref — the journal
/// dedups it to a single fact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriticVerdict {
    /// The producer output passed the deterministic check.
    Valid,
    /// The producer output failed; `reason` names the closed failure kind.
    Invalid {
        /// Why the check rejected the output.
        reason: CriticReason,
    },
}

/// The closed failure vocabulary — one variant family per deterministic check
/// kind, plus the total-input `Unparseable` escape hatch.
///
/// Illegal states unrepresentable: a reason is exactly one kind, and each
/// carries only **deterministic, reproducible, integer-scaled** evidence
/// (indices / counts / byte offsets) — never floats, never host-derived data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriticReason {
    /// The schema check failed: the output did not conform to the declared tag.
    SchemaMismatch {
        /// The schema the spec required.
        expected: SchemaTag,
        /// The first failing structural fact.
        detail: SchemaFault,
    },
    /// The dedup check failed: duplicate records were detected.
    DuplicateDetected {
        /// Count of records that duplicated an earlier one (occurrences beyond
        /// the first).
        duplicate_count: u64,
        /// Zero-based index of the FIRST record that duplicated an earlier one.
        first_duplicate_index: u64,
    },
    /// The stat-bounds check failed: an aggregate left its declared inclusive
    /// integer bound.
    StatOutOfBounds {
        /// Which statistic was out of bounds.
        stat: StatKind,
        /// The integer-encoded observed value (see `StatBoundsSpec::scale`).
        observed_scaled: i64,
        /// The inclusive lower bound the spec declared (scaled).
        lo_scaled: i64,
        /// The inclusive upper bound the spec declared (scaled).
        hi_scaled: i64,
    },
    /// The PII-leakage check failed: a forbidden pattern class matched.
    PiiLeak {
        /// Which detector class matched.
        class: PiiClass,
        /// Byte offset of the first match in the input.
        match_offset: u64,
        /// Byte length of the matched span.
        match_len: u64,
    },
    /// The input bytes could not be parsed into the structure the check needs
    /// (e.g. a dedup/stat over a malformed record framing). Total-on-adversarial
    /// input: evaluation NEVER panics — a parse failure is a deterministic
    /// `Invalid`, not a crash.
    Unparseable {
        /// Which check raised it.
        check: CheckKind,
        /// Byte offset where parsing failed.
        at_offset: u64,
    },
}

/// Stable, u8-tagged discriminant naming a deterministic check kind. Carried by
/// [`CriticReason::Unparseable`] and produced by `CheckSpec::kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CheckKind {
    /// The schema check.
    Schema,
    /// The dedup check.
    Dedup,
    /// The stat-bounds check.
    StatBounds,
    /// The PII-leakage check.
    PiiLeak,
}

/// The first structural fact that failed a schema check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaFault {
    /// The top-level schema tag did not match (e.g. expected `Json`, got bytes
    /// that are not a well-formed JSON document).
    TagMismatch,
    /// A `Tensor` / `Vector` payload's byte length did not match the declared
    /// element count.
    ShapeMismatch {
        /// Element count the schema declared.
        expected_elems: u64,
        /// Actual byte length of the payload.
        actual_bytes: u64,
    },
    /// `Text` / `Json` required UTF-8 and the bytes were not valid UTF-8.
    NotUtf8 {
        /// Byte offset of the first invalid UTF-8 sequence.
        at_offset: u64,
    },
    /// `Json` required a well-formed JSON document and parsing failed.
    NotJson {
        /// Byte offset where JSON parsing failed (0 if not localizable).
        at_offset: u64,
    },
}

/// Which aggregate statistic a stat-bounds check computes over the records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StatKind {
    /// The number of records.
    RecordCount,
    /// The arithmetic mean of each record's numeric field (scaled integer,
    /// integer division — see `StatBoundsSpec`).
    MeanScaled,
    /// The minimum of each record's numeric field (scaled integer).
    MinScaled,
    /// The maximum of each record's numeric field (scaled integer).
    MaxScaled,
}

/// A forbidden PII detector class. Iteration order over a `BTreeSet<PiiClass>`
/// (canonical, `Ord`-derived below) is the deterministic multi-match priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PiiClass {
    /// An email address.
    Email,
    /// An IPv4 address in dotted-quad form.
    IpV4,
    /// A credit-card number passing the Luhn checksum.
    CreditCardLuhn,
    /// A US Social Security Number (NNN-NN-NNNN).
    UsSsn,
}

impl CriticVerdict {
    /// `true` iff this is [`CriticVerdict::Valid`].
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }

    /// Canonical content-addressable bytes:
    /// `[CRITIC_SCHEMA_VERSION as u16 LE] ‖ bincode(self, canonical_config())`.
    ///
    /// Infallible: there are no floats and no non-encodable variants, so the
    /// bincode encode cannot fail (same precondition as `kx_mote::MoteDef::hash`).
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        // SAFETY (expect): CriticVerdict has no floats and no non-encodable
        // types, so canonical bincode encoding is infallible — mirrors the
        // documented-infallible `kx_mote::MoteDef::hash` encode site.
        let body = bincode::serde::encode_to_vec(self, canonical_config()).expect(
            "CriticVerdict canonical encoding is infallible (no floats, no non-encodable types)",
        );
        let mut out = Vec::with_capacity(2 + body.len());
        out.extend_from_slice(&CRITIC_SCHEMA_VERSION.to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decode canonical bytes produced by [`CriticVerdict::encode`].
    ///
    /// # Errors
    ///
    /// [`VerdictDecodeError::UnknownSchemaVersion`] if the two-byte version
    /// prefix is not [`CRITIC_SCHEMA_VERSION`]; [`VerdictDecodeError::Malformed`]
    /// if the prefix is missing or the body fails to decode.
    pub fn decode(bytes: &[u8]) -> Result<Self, VerdictDecodeError> {
        let Some((prefix, body)) = bytes.split_first_chunk::<2>() else {
            return Err(VerdictDecodeError::Malformed);
        };
        let version = u16::from_le_bytes(*prefix);
        if version != CRITIC_SCHEMA_VERSION {
            return Err(VerdictDecodeError::UnknownSchemaVersion(version));
        }
        let (verdict, consumed) =
            bincode::serde::decode_from_slice::<Self, _>(body, canonical_config())
                .map_err(|_| VerdictDecodeError::Malformed)?;
        // Reject trailing garbage — canonical bytes are exact.
        if consumed != body.len() {
            return Err(VerdictDecodeError::Malformed);
        }
        Ok(verdict)
    }

    /// The content-addressed identity of this verdict: `blake3(self.encode())`.
    ///
    /// The executor commits the encoded verdict to the content store; this is
    /// the 32-byte ref both the executor and the projection compute, so they
    /// agree byte-for-byte (SN-8 exact equality).
    #[must_use]
    pub fn content_ref_bytes(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
}

/// The canonical bincode configuration for [`CriticVerdict`] / `CheckSpec`
/// encodings: bincode v2, little-endian, fixed-int. **Byte-identical to
/// `kx_mote::canonical_config`** — replicated here so this crate stays
/// `kx-mote`-free. Any change to these flags is a [`CRITIC_SCHEMA_VERSION`] bump.
#[must_use]
pub fn canonical_config(
) -> bincode::config::Configuration<bincode::config::LittleEndian, bincode::config::Fixint> {
    bincode::config::standard()
        .with_little_endian()
        .with_fixed_int_encoding()
}

/// Failure decoding a [`CriticVerdict`] from canonical bytes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VerdictDecodeError {
    /// The version prefix named a `CRITIC_SCHEMA_VERSION` this build does not
    /// understand.
    #[error("unknown CriticVerdict schema_version {0}")]
    UnknownSchemaVersion(u16),
    /// The bytes were missing the version prefix, failed to decode, or carried
    /// trailing garbage.
    #[error("malformed CriticVerdict body")]
    Malformed,
}
