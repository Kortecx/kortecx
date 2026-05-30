//! The **PII-leakage** check: does a forbidden detector class match the output?
//!
//! Each class is a deterministic byte scanner (`regex::bytes`, leftmost-first,
//! compiled once via `LazyLock`). When several forbidden classes match, the one
//! whose first match has the smallest byte offset wins; ties break by the
//! `BTreeSet<PiiClass>` iteration order (canonical priority). The detectors run
//! over raw bytes, so non-UTF-8 input is handled without a panic.

use std::sync::LazyLock;

use kx_critic_types::{CriticReason, CriticVerdict, PiiClass, PiiSpec};
use regex::bytes::Regex;

/// Evaluate the PII-leakage check. Total + deterministic.
#[must_use]
pub fn eval(spec: &PiiSpec, input: &[u8]) -> CriticVerdict {
    // Iterate forbidden classes in BTreeSet (canonical) order; keep the match
    // with the smallest byte offset, ties resolved by this iteration order.
    let mut best: Option<(PiiClass, usize, usize)> = None;
    for &class in &spec.forbidden {
        if let Some((start, len)) = find_first(class, input) {
            let take = match best {
                Some((_, best_start, _)) => start < best_start,
                None => true,
            };
            if take {
                best = Some((class, start, len));
            }
        }
    }

    match best {
        None => CriticVerdict::Valid,
        Some((class, start, len)) => CriticVerdict::Invalid {
            reason: CriticReason::PiiLeak {
                class,
                match_offset: start as u64,
                match_len: len as u64,
            },
        },
    }
}

fn find_first(class: PiiClass, input: &[u8]) -> Option<(usize, usize)> {
    match class {
        PiiClass::Email => first_match(&EMAIL, input),
        PiiClass::IpV4 => first_match(&IPV4, input),
        PiiClass::UsSsn => first_match(&US_SSN, input),
        PiiClass::CreditCardLuhn => {
            // A candidate digit run that ALSO passes the Luhn checksum. Take the
            // earliest such run (find_iter yields matches left-to-right).
            for m in CC_CANDIDATE.find_iter(input) {
                if luhn_ok(m.as_bytes()) {
                    return Some((m.start(), m.end() - m.start()));
                }
            }
            None
        }
    }
}

fn first_match(re: &Regex, input: &[u8]) -> Option<(usize, usize)> {
    re.find(input).map(|m| (m.start(), m.end() - m.start()))
}

/// Luhn checksum over an ASCII digit string. `digits` is assumed to be all
/// `b'0'..=b'9'` (the candidate regex guarantees this).
fn luhn_ok(digits: &[u8]) -> bool {
    let mut sum: u32 = 0;
    // Walk right-to-left, doubling every second digit.
    for (i, &b) in digits.iter().rev().enumerate() {
        let mut d = u32::from(b - b'0');
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum.is_multiple_of(10)
}

// Patterns compiled once. `expect` is on a frozen literal pattern, so it cannot
// fail at runtime — a malformed pattern is a compile-time-detectable test
// failure, not a production path.
#[allow(clippy::expect_used)]
fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).expect("frozen PII pattern is a valid regex")
}

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| compile(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}"));

static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"\b(?:(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9][0-9]|[1-9]?[0-9])\b",
    )
});

static US_SSN: LazyLock<Regex> = LazyLock::new(|| compile(r"\b[0-9]{3}-[0-9]{2}-[0-9]{4}\b"));

static CC_CANDIDATE: LazyLock<Regex> = LazyLock::new(|| compile(r"\b[0-9]{13,19}\b"));
