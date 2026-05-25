#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]

//! # kx-normalizer — deterministic canonicalization BEFORE fingerprinting
//!
//! The normalizer is the controlled escape hatch for "two inputs that mean
//! the same thing but aren't bit-identical" — exactly the situation where a
//! lesser system would reach for fuzzy matching. **The fix is NOT fuzzy
//! matching.** Per SN-8 ("model proposes, runtime enforces") and D33, the
//! runtime serves cache hits only on EXACT cryptographic equality. Two
//! Motes match iff their derived `MoteId`s match bit-for-bit. There is NO
//! similarity operator anywhere on the identity path.
//!
//! The normalizer reconciles this with the workflow author's reasonable
//! expectation that `"ls   -la"` and `"ls -la"` should refer to the same
//! command. It does so by **canonicalizing inputs BEFORE they feed into
//! `MoteId` derivation** — the two inputs become bit-identical after the
//! rule applies, and the memoizer's exact-equality hit then fires for
//! free. The fingerprint is computed over the canonical form.
//!
//! ## Two normalizer kinds (D33)
//!
//! - [`NormalizerKind::DeterministicRule`] — a versioned rule set, applied
//!   purely. Bit-identical across machines + replays per `(rule_set,
//!   version)`. **v0.1 ships ONE rule set**: [`RuleSet::CommandLineIntent`]
//!   v1, which canonicalizes whitespace (trim + collapse internal
//!   whitespace runs to a single ASCII space). Conservative — does NOT
//!   reorder flags, does NOT resolve paths, does NOT canonicalize
//!   quoting. **When the rule is unsure, it keeps inputs distinct.**
//!
//! - [`NormalizerKind::ModelAsMote`] — the model-as-Mote seam for fuzzy
//!   residue. The normalizer IS itself a Mote whose canonical output is
//!   content-addressed and replayed. This is the escape hatch when a
//!   deterministic rule cannot encode the intent; the model's
//!   canonicalization is frozen as a durable fact.
//!
//! ## Why "conservative" matters
//!
//! A normalizer that merges things it shouldn't is far worse than one that
//! keeps too many things distinct. Merging silently routes cache hits to
//! the wrong upstream — a correctness bug that surfaces only under
//! workflow author surprise. Keeping things distinct surfaces as "the
//! cache didn't fire" — diagnosable, addressable, never silently wrong.
//!
//! v0.1's `CommandLineIntent` v1 is intentionally tiny: trim + whitespace
//! collapse only. No flag-reordering (would silently change `cp src dst`
//! into `cp dst src` if applied naively). No path normalization (changes
//! semantics under symlinks). No quoting normalization (requires shell
//! parser). Each of these is a **deliberate future-rule slot** — when
//! added, they bump the rule version, never silently change v1's
//! behavior.
//!
//! ## Versioning
//!
//! `(rule_set, version)` is the dispatch key. Two callers with the same
//! `(rule_set, version)` get bit-identical canonical bytes; replay
//! reproduces verbatim. **Future rule additions MUST bump the version**
//! — silently changing v1's canonical output would break replay for
//! every MoteId derived from a v1-normalized input.
//!
//! ## What lives here
//!
//! - [`RuleSet`] — the versioned rule-set identifier enum.
//! - [`NormalizerKind`] — the per-call dispatch discriminant.
//! - [`NormalizerError`] — typed failures.
//! - [`normalize_deterministic`] — apply a rule set + version to raw bytes.
//!
//! ## What does NOT live here
//!
//! - The fingerprint computation itself (that's `kx_mote::derive_mote_id`).
//! - The journal-write side of "ModelAsMote" normalizers (the executor at
//!   P1.9 commits the model's canonical output as a Mote).
//! - Any similarity operator. **Forbidden by SN-8** anywhere on the
//!   identity path.

use bytes::Bytes;
use kx_mote::MoteId;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// RuleSet — versioned identifier of a deterministic canonicalization rule
// ---------------------------------------------------------------------------

/// Identifier for a deterministic canonicalization rule set.
///
/// Two calls to [`normalize_deterministic`] with the same `(RuleSet,
/// version)` produce bit-identical canonical bytes — across machines,
/// across replays, across process restarts.
///
/// **v0.1 ships exactly one variant**: [`RuleSet::CommandLineIntent`].
/// Future variants extend this enum; the canonical-classifier-cannot-drift
/// pattern (STEP 6.2 from PR 4.5) governs the proptest strategy update.
///
/// # Examples
///
/// ```
/// use kx_normalizer::RuleSet;
///
/// // RuleSet is small + Copy; cheap to pass by value.
/// let rs = RuleSet::CommandLineIntent;
/// assert_eq!(rs, RuleSet::CommandLineIntent);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RuleSet {
    /// Canonicalize command-line-intent text.
    ///
    /// **v1 ships exactly one rule**: trim leading/trailing whitespace
    /// AND collapse internal whitespace runs (any Unicode whitespace) to
    /// a single ASCII space. UTF-8 only.
    ///
    /// Deliberately conservative:
    /// - Does NOT reorder flags (would silently break `cp src dst` if
    ///   applied naively).
    /// - Does NOT resolve `./` / `../` in paths (changes semantics under
    ///   symlinks).
    /// - Does NOT canonicalize quoting (requires a shell parser).
    ///
    /// Each of those is a deliberate future-rule slot. When added, they
    /// bump the version — they NEVER silently change v1's behavior.
    CommandLineIntent,
}

impl RuleSet {
    /// The highest version this build supports for `self`. Increments when
    /// a new rule lands on the same rule set.
    #[must_use]
    pub const fn current_version(self) -> u32 {
        match self {
            Self::CommandLineIntent => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// NormalizerKind — per-call dispatch discriminant
// ---------------------------------------------------------------------------

/// Per-call dispatch discriminant: which normalizer (if any) was applied
/// to the raw input before `MoteId` derivation.
///
/// Stored in the workflow author's submission spec so replay knows how to
/// reproduce the canonical form bit-for-bit.
///
/// # Examples
///
/// ```
/// use kx_normalizer::{NormalizerKind, RuleSet};
///
/// // Deterministic rule: replay reproduces from (rule_set, version).
/// let det = NormalizerKind::DeterministicRule {
///     rule_set: RuleSet::CommandLineIntent,
///     version: 1,
/// };
///
/// // Model-as-Mote: replay reads the cached canonical output via mote_id.
/// // (The MoteId here is illustrative.)
/// let model = NormalizerKind::ModelAsMote {
///     mote_id: kx_mote::MoteId::from_bytes([0; 32]),
/// };
///
/// assert_ne!(det, model);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NormalizerKind {
    /// A deterministic rule applied. Replay needs `(rule_set, version)`
    /// to reproduce the canonical form bit-for-bit.
    DeterministicRule {
        /// The rule set identifier.
        rule_set: RuleSet,
        /// The rule set version (>= 1).
        version: u32,
    },

    /// The normalizer IS a Mote — its canonical output is content-
    /// addressed and replayed. Escape hatch for fuzzy residue per D33.
    /// Replay reads the cached canonical output via `mote_id`; the
    /// executor at P1.9 commits the model's canonical output as a Mote.
    ModelAsMote {
        /// The MoteId of the normalizer-Mote whose committed result_ref
        /// is the canonical form.
        mote_id: MoteId,
    },
}

// ---------------------------------------------------------------------------
// NormalizerError — typed failures
// ---------------------------------------------------------------------------

/// Errors returned by [`normalize_deterministic`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NormalizerError {
    /// The requested `(rule_set, version)` is not implemented in this
    /// build. Most often hit when replay encounters a version newer
    /// than the current binary supports.
    #[error("rule set {rule_set:?} version {version} is not implemented (this build supports versions 1..={max_supported})")]
    UnsupportedVersion {
        /// The requested rule set.
        rule_set: RuleSet,
        /// The requested version.
        version: u32,
        /// The highest version this build supports for `rule_set`.
        max_supported: u32,
    },

    /// The input bytes are not valid UTF-8. The current rule sets all
    /// require UTF-8 text; this is a precondition violation.
    #[error("input is not valid UTF-8 (rule set requires UTF-8 text)")]
    InvalidUtf8,
}

// ---------------------------------------------------------------------------
// normalize_deterministic — the dispatch entry point
// ---------------------------------------------------------------------------

/// Apply a deterministic rule set to raw `input` bytes, returning the
/// canonical form.
///
/// Pure / total / deterministic per `(rule_set, version)`: two calls with
/// identical inputs produce bit-identical outputs across machines + replays
/// + process restarts.
///
/// # Versioning contract
///
/// `(rule_set, version)` is the dispatch key. Future rule additions MUST
/// bump the version — silently changing an existing version's canonical
/// output would break replay for every `MoteId` derived from a previously-
/// normalized input. Replay encountering a version higher than the binary
/// supports returns [`NormalizerError::UnsupportedVersion`].
///
/// # Errors
///
/// - [`NormalizerError::UnsupportedVersion`] — `(rule_set, version)` not
///   implemented in this build.
/// - [`NormalizerError::InvalidUtf8`] — input is not valid UTF-8 (current
///   rule sets all require UTF-8 text).
///
/// # Examples
///
/// ```
/// use kx_normalizer::{normalize_deterministic, RuleSet};
///
/// let raw = b"  ls   -la   ";
/// let canon = normalize_deterministic(RuleSet::CommandLineIntent, 1, raw).unwrap();
/// assert_eq!(&canon[..], b"ls -la");
///
/// // Same canonical form for any equivalent whitespace variant.
/// let raw2 = b"\tls\n-la\t\t";
/// let canon2 = normalize_deterministic(RuleSet::CommandLineIntent, 1, raw2).unwrap();
/// assert_eq!(canon, canon2);
/// ```
#[tracing::instrument(level = "debug", skip(input), fields(rule_set = ?rule_set, version = version, input_len = input.len()))]
pub fn normalize_deterministic(
    rule_set: RuleSet,
    version: u32,
    input: &[u8],
) -> Result<Bytes, NormalizerError> {
    // Exhaustive match on RuleSet — adding a new variant is a compile
    // error, NOT a silent wildcard match. This is the
    // canonical-classifier-cannot-drift contract at the code level (mirror
    // of STEP 6.2 + the `arb_rule_set` proptest strategy at the test
    // level). Per-variant inner match dispatches on version.
    match rule_set {
        RuleSet::CommandLineIntent => match version {
            1 => command_line_intent_v1(input),
            _ => Err(NormalizerError::UnsupportedVersion {
                rule_set,
                version,
                max_supported: rule_set.current_version(),
            }),
        },
    }
}

// ---------------------------------------------------------------------------
// CommandLineIntent v1 — the v0.1 rule
// ---------------------------------------------------------------------------

/// Trim + collapse-internal-whitespace canonicalization.
///
/// Algorithm:
/// 1. Decode `input` as UTF-8 (error if invalid).
/// 2. Trim leading/trailing whitespace (Unicode `White_Space` property).
/// 3. Collapse internal runs of whitespace to a single ASCII space.
/// 4. Encode back to UTF-8 bytes.
///
/// **What v1 does NOT do** (each is a deliberate future-rule slot):
/// - Reorder flags (`ls -la` vs `ls -al` — sorting `-la` would silently
///   break commands where argument order matters, e.g., `cp src dst`).
/// - Resolve `./` / `../` in paths (changes semantics under symlinks).
/// - Canonicalize quoting (`'foo'` vs `"foo"` — requires shell parser).
/// - Normalize variable interpolation (`$HOME` vs `${HOME}` — env-dependent).
///
/// **Idempotence**: `f(f(x)) == f(x)` for all `x`. Verified by proptest.
fn command_line_intent_v1(input: &[u8]) -> Result<Bytes, NormalizerError> {
    let s = std::str::from_utf8(input).map_err(|_| NormalizerError::InvalidUtf8)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(Bytes::new());
    }
    // Walk the trimmed string char-by-char, emitting non-whitespace
    // verbatim and collapsing runs of whitespace to a single ASCII space.
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_was_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_was_ws {
                out.push(' ');
                prev_was_ws = true;
            }
        } else {
            out.push(ch);
            prev_was_ws = false;
        }
    }
    Ok(Bytes::from(out.into_bytes()))
}

// ---------------------------------------------------------------------------
// Inline tests — fixture-heavy tests live in tests/proptest_normalizer.rs
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_for_command_line_intent_is_one() {
        assert_eq!(RuleSet::CommandLineIntent.current_version(), 1);
    }

    #[test]
    fn unsupported_version_returns_typed_error() {
        let err = normalize_deterministic(RuleSet::CommandLineIntent, 999, b"hi").unwrap_err();
        match err {
            NormalizerError::UnsupportedVersion {
                rule_set,
                version,
                max_supported,
            } => {
                assert_eq!(rule_set, RuleSet::CommandLineIntent);
                assert_eq!(version, 999);
                assert_eq!(max_supported, 1);
            }
            NormalizerError::InvalidUtf8 => panic!("expected UnsupportedVersion, got InvalidUtf8"),
        }
    }

    #[test]
    fn version_zero_returns_unsupported() {
        let err = normalize_deterministic(RuleSet::CommandLineIntent, 0, b"hi").unwrap_err();
        assert!(matches!(err, NormalizerError::UnsupportedVersion { .. }));
    }

    #[test]
    fn invalid_utf8_returns_typed_error() {
        // 0xFF is invalid UTF-8.
        let err =
            normalize_deterministic(RuleSet::CommandLineIntent, 1, &[0xFF, 0xFE]).unwrap_err();
        assert_eq!(err, NormalizerError::InvalidUtf8);
    }

    #[test]
    fn normalizer_kind_variants_are_distinct() {
        let det = NormalizerKind::DeterministicRule {
            rule_set: RuleSet::CommandLineIntent,
            version: 1,
        };
        let model = NormalizerKind::ModelAsMote {
            mote_id: MoteId::from_bytes([0; 32]),
        };
        assert_ne!(det, model);
    }
}
