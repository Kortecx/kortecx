//! The **dedup** check: are there duplicate records under a declared framing +
//! key? Frame the input, extract each record's key (whole record or a byte
//! sub-range), and reject on the first key that repeats an earlier one.

use std::collections::BTreeSet;

use kx_critic_types::{CheckKind, CriticReason, CriticVerdict, DedupSpec};

use crate::framing::{frame, key_of};

/// Evaluate the dedup check. Total + deterministic.
#[must_use]
pub fn eval(spec: &DedupSpec, input: &[u8]) -> CriticVerdict {
    let records = match frame(spec.framing, input) {
        Ok(r) => r,
        Err(e) => return unparseable(e.at_offset),
    };

    let mut seen: BTreeSet<&[u8]> = BTreeSet::new();
    let mut first_duplicate_index: Option<u64> = None;
    let mut duplicate_count: u64 = 0;

    for (idx, record) in records.iter().enumerate() {
        let Some(key) = key_of(record, spec.key_range) else {
            // Key range out of bounds for this record → deterministic Unparseable.
            // Offset points at the record's place in the stream by index; we use
            // the index as a stable locator (byte offset is framing-dependent).
            return unparseable(idx as u64);
        };
        if seen.contains(key) {
            duplicate_count += 1;
            if first_duplicate_index.is_none() {
                first_duplicate_index = Some(idx as u64);
            }
        } else {
            seen.insert(key);
        }
    }

    match first_duplicate_index {
        None => CriticVerdict::Valid,
        Some(first_duplicate_index) => CriticVerdict::Invalid {
            reason: CriticReason::DuplicateDetected {
                duplicate_count,
                first_duplicate_index,
            },
        },
    }
}

fn unparseable(at_offset: u64) -> CriticVerdict {
    CriticVerdict::Invalid {
        reason: CriticReason::Unparseable {
            check: CheckKind::Dedup,
            at_offset,
        },
    }
}
