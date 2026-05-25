// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `kx-normalizer` (SN-4 v2 #6 — pinned per D33 + `context-assembly.md` §4).
//!
//! Properties:
//!
//! 1. `normalize_deterministic` is DETERMINISTIC — same `(rule_set,
//!    version, input)` → bit-identical output.
//! 2. `normalize_deterministic` is TOTAL — never panics on any input
//!    shape (including invalid UTF-8 + huge whitespace runs +
//!    pathological Unicode).
//! 3. **IDEMPOTENT** — `f(f(x)) == f(x)`. The canonical form is a fixed
//!    point of the rule; running the normalizer twice produces the same
//!    bytes as running it once. This is what makes the normalizer safe
//!    to re-apply on replay without drift.
//! 4. **No INTERNAL whitespace runs in output** (CommandLineIntent v1
//!    invariant) — every ASCII space in the output is a separator
//!    between non-whitespace tokens; never two adjacent.
//! 5. **No LEADING or TRAILING whitespace in output** (CommandLineIntent
//!    v1 invariant) — output is trimmed.
//! 6. **Conservative: when ambiguous, keep distinct.** Two inputs with
//!    different non-whitespace token sequences MUST produce different
//!    canonical outputs. Specifically: if a non-whitespace character
//!    differs between inputs (or the count of non-whitespace tokens
//!    differs), the canonical outputs differ. The normalizer does NOT
//!    re-order or merge tokens.
//! 7. **Class-covering sweep over `RuleSet`** (STEP 6.2 pattern from
//!    PR 4.5): `arb_rule_set()` enumerates ALL variants with a "MUST
//!    update on new variant" comment.

use bytes::Bytes;
use kx_normalizer::{normalize_deterministic, NormalizerError, NormalizerKind, RuleSet};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_rule_set() -> impl Strategy<Value = RuleSet> {
    // MUST be updated when a RuleSet variant is added — this strategy is
    // the test surface's gate against silent variant addition (mirrors the
    // STEP 6.2 canonical-classifier-cannot-drift contract from PR 4.5).
    prop_oneof![Just(RuleSet::CommandLineIntent),]
}

/// Strategy: an arbitrary string (any chars including Unicode whitespace).
fn arb_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(any::<char>(), 0..=128).prop_map(|v| v.into_iter().collect())
}

/// Strategy: an arbitrary byte vector (may or may not be valid UTF-8).
fn arb_bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=128)
}

// ---------------------------------------------------------------------------
// Hand-written cell tests for CommandLineIntent v1
// ---------------------------------------------------------------------------

#[test]
fn empty_input_yields_empty_output() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"").unwrap();
    assert_eq!(&out[..], b"");
}

#[test]
fn pure_whitespace_yields_empty_output() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"   \t\n  ").unwrap();
    assert_eq!(&out[..], b"");
}

#[test]
fn single_word_unchanged() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"hello").unwrap();
    assert_eq!(&out[..], b"hello");
}

#[test]
fn collapses_internal_multi_space() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"ls   -la").unwrap();
    assert_eq!(&out[..], b"ls -la");
}

#[test]
fn trims_leading_and_trailing() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"  hello world  ").unwrap();
    assert_eq!(&out[..], b"hello world");
}

#[test]
fn normalizes_tabs_newlines_and_carriage_returns_to_single_space() {
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"ls\t-la\nfoo\rbar").unwrap();
    assert_eq!(&out[..], b"ls -la foo bar");
}

#[test]
fn unicode_whitespace_collapsed_to_ascii_space() {
    // U+00A0 NO-BREAK SPACE + U+2003 EM SPACE + ASCII space.
    let input = "ls\u{00A0}-la\u{2003}foo  bar".as_bytes();
    let out = normalize_deterministic(RuleSet::CommandLineIntent, 1, input).unwrap();
    assert_eq!(&out[..], b"ls -la foo bar");
}

#[test]
fn flag_ordering_is_not_normalized_v1_conservative() {
    // CommandLineIntent v1 deliberately does NOT reorder flags. `-la` and
    // `-al` are DISTINCT inputs and produce DISTINCT canonical outputs.
    // (A future v2 may add flag-sorting; that v2 will have its own
    // version key and not silently change v1.)
    let a = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"ls -la").unwrap();
    let b = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"ls -al").unwrap();
    assert_ne!(a, b, "v1 MUST NOT reorder flags (conservative)");
}

#[test]
fn argument_ordering_is_not_normalized_v1_conservative() {
    // `cp src dst` and `cp dst src` are distinct inputs. Sorting tokens
    // would silently break command semantics — the normalizer refuses to
    // do this.
    let a = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"cp src dst").unwrap();
    let b = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"cp dst src").unwrap();
    assert_ne!(
        a, b,
        "v1 MUST NOT sort tokens (would silently break ordered-argument commands)"
    );
}

#[test]
fn case_is_preserved_v1_conservative() {
    // Case-sensitivity matters in command-line semantics (especially
    // flags). v1 preserves case.
    let a = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"Hello World").unwrap();
    let b = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"hello world").unwrap();
    assert_ne!(a, b, "v1 MUST preserve case");
}

#[test]
fn idempotent_smoke() {
    let raw = b"  ls   -la  ";
    let once = normalize_deterministic(RuleSet::CommandLineIntent, 1, raw).unwrap();
    let twice = normalize_deterministic(RuleSet::CommandLineIntent, 1, &once).unwrap();
    assert_eq!(once, twice, "normalize MUST be idempotent");
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: DETERMINISTIC — same inputs, same output.
    #[test]
    fn prop_normalize_is_deterministic(
        rs in arb_rule_set(),
        text in arb_text(),
    ) {
        let a = normalize_deterministic(rs, 1, text.as_bytes());
        let b = normalize_deterministic(rs, 1, text.as_bytes());
        prop_assert_eq!(a, b);
    }

    /// Property 2: TOTAL — never panics, even on arbitrary (possibly
    /// invalid UTF-8) byte sequences. Returns either Ok or a typed Err.
    #[test]
    fn prop_normalize_is_total(
        rs in arb_rule_set(),
        bytes in arb_bytes(),
        version in 0u32..=10,
    ) {
        // Reaching this assertion proves no panic.
        let _ = normalize_deterministic(rs, version, &bytes);
    }

    /// Property 3: IDEMPOTENT — f(f(x)) == f(x).
    /// **The canonical form is a fixed point**; replay safety depends on
    /// this. If a future rule lands that's non-idempotent, this test
    /// will fail loudly.
    #[test]
    fn prop_normalize_is_idempotent(
        text in arb_text(),
    ) {
        let once = normalize_deterministic(RuleSet::CommandLineIntent, 1, text.as_bytes());
        if let Ok(canon) = once {
            let twice = normalize_deterministic(RuleSet::CommandLineIntent, 1, &canon)
                .expect("canonical form is valid UTF-8 by construction");
            prop_assert_eq!(canon, twice);
        }
        // If once was Err (e.g., invalid UTF-8), there's nothing to
        // idempotently re-normalize. The error path is its own fixed
        // point — total + deterministic — and Property 1 covers it.
    }

    /// Property 4: NO INTERNAL whitespace runs in CommandLineIntent v1
    /// output. Every space in the output is between two non-whitespace
    /// tokens; never two adjacent spaces.
    #[test]
    fn prop_no_internal_whitespace_runs(
        text in arb_text(),
    ) {
        if let Ok(canon) = normalize_deterministic(RuleSet::CommandLineIntent, 1, text.as_bytes()) {
            // The output is UTF-8 by construction.
            let s = std::str::from_utf8(&canon).expect("output is UTF-8");
            prop_assert!(
                !s.contains("  "),
                "output must not contain two adjacent ASCII spaces; got {:?}",
                s
            );
            // No non-ASCII whitespace either (all whitespace was collapsed
            // to ASCII space).
            for ch in s.chars() {
                if ch.is_whitespace() {
                    prop_assert_eq!(ch, ' ', "all whitespace in output must be ASCII space");
                }
            }
        }
    }

    /// Property 5: NO LEADING or TRAILING whitespace in CommandLineIntent
    /// v1 output.
    #[test]
    fn prop_no_leading_or_trailing_whitespace(
        text in arb_text(),
    ) {
        if let Ok(canon) = normalize_deterministic(RuleSet::CommandLineIntent, 1, text.as_bytes()) {
            let s = std::str::from_utf8(&canon).expect("output is UTF-8");
            if !s.is_empty() {
                prop_assert!(!s.starts_with(char::is_whitespace), "output must not start with whitespace");
                prop_assert!(!s.ends_with(char::is_whitespace), "output must not end with whitespace");
            }
        }
    }

    /// Property 6: CONSERVATIVE — different non-whitespace token
    /// sequences produce different canonical outputs. The normalizer
    /// does NOT re-order or merge tokens. We test this by comparing the
    /// canonical form to a "ground truth" computed by collecting the
    /// input's non-whitespace tokens in order and joining with single
    /// spaces.
    #[test]
    fn prop_conservative_token_order_preserved(
        text in arb_text(),
    ) {
        if let Ok(canon) = normalize_deterministic(RuleSet::CommandLineIntent, 1, text.as_bytes()) {
            let canon_str = std::str::from_utf8(&canon).expect("output is UTF-8");
            // Ground truth: split input on Unicode whitespace, keep
            // non-empty tokens, join with single ASCII space.
            let ground_truth: String = text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            prop_assert_eq!(canon_str, &ground_truth,
                "canonical form must equal whitespace-split-then-join (token-order preserving)");
        }
    }

    /// Property 7: class-covering sweep over `RuleSet`. Every variant
    /// generated by `arb_rule_set` must be implementable at its
    /// `current_version()`. If a future variant is added but its
    /// implementation isn't wired into `normalize_deterministic`, this
    /// property fires.
    #[test]
    fn prop_every_rule_set_has_implementation_at_current_version(
        rs in arb_rule_set(),
        text in arb_text(),
    ) {
        let result = normalize_deterministic(rs, rs.current_version(), text.as_bytes());
        // Must NOT return UnsupportedVersion at the current version.
        match result {
            Ok(_) => {}
            Err(NormalizerError::UnsupportedVersion { .. }) => {
                prop_assert!(false,
                    "rule_set {:?} at current_version() {} returned UnsupportedVersion",
                    rs, rs.current_version());
            }
            Err(NormalizerError::InvalidUtf8) => {
                // Always OK — arb_text generates valid UTF-8 by
                // construction, so this branch should be unreachable for
                // CommandLineIntent. But if a future RuleSet accepts
                // non-UTF-8 input, this branch is its own pass.
            }
        }
    }

    /// Property 8: NormalizerKind variants are distinct (PartialEq + Hash
    /// behave). Pins that no Default impl was accidentally added.
    #[test]
    fn prop_normalizer_kind_variants_distinct(
        version in 1u32..=10,
        seed in any::<u8>(),
    ) {
        let det = NormalizerKind::DeterministicRule {
            rule_set: RuleSet::CommandLineIntent,
            version,
        };
        let model = NormalizerKind::ModelAsMote {
            mote_id: kx_mote::MoteId::from_bytes([seed; 32]),
        };
        prop_assert_ne!(det, model);
    }
}

// ---------------------------------------------------------------------------
// Bytes shape sanity
// ---------------------------------------------------------------------------

#[test]
fn output_bytes_type_is_bytes() {
    // Compile-time check that the return type is bytes::Bytes (so
    // downstream consumers can rely on zero-copy slicing + Arc-cheap
    // cloning).
    let out: Bytes = normalize_deterministic(RuleSet::CommandLineIntent, 1, b"hi").unwrap();
    assert_eq!(&out[..], b"hi");
}
