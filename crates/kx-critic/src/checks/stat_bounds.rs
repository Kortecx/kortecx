//! The **stat-bounds** check: is a declared aggregate within an inclusive integer
//! bound? Frame the input, parse each record's numeric field as a scaled integer
//! (ASCII decimal — never a float), compute the statistic in scaled-integer
//! space, and reject if it leaves `[lo_scaled, hi_scaled]`.
//!
//! Aggregates over an empty record set are defined (not a panic): `RecordCount`
//! is `0`; `MeanScaled` / `MinScaled` / `MaxScaled` are `0`. `MeanScaled` uses
//! integer division (truncation toward zero).

use kx_critic_types::{CheckKind, CriticReason, CriticVerdict, StatBoundsSpec, StatKind};

use crate::framing::{frame, key_of};

/// Evaluate the stat-bounds check. Total + deterministic.
#[must_use]
pub fn eval(spec: &StatBoundsSpec, input: &[u8]) -> CriticVerdict {
    let records = match frame(spec.framing, input) {
        Ok(r) => r,
        Err(e) => return unparseable(e.at_offset),
    };

    let observed_scaled = match spec.stat {
        StatKind::RecordCount => i64::try_from(records.len()).unwrap_or(i64::MAX),
        StatKind::MeanScaled | StatKind::MinScaled | StatKind::MaxScaled => {
            let mut sum: i128 = 0;
            let mut min: Option<i64> = None;
            let mut max: Option<i64> = None;
            for (idx, record) in records.iter().enumerate() {
                let Some(field) = key_of(record, spec.numeric_field_range) else {
                    return unparseable(idx as u64);
                };
                let Some(value) = parse_scaled_int(field) else {
                    return unparseable(idx as u64);
                };
                sum += i128::from(value);
                min = Some(min.map_or(value, |m| m.min(value)));
                max = Some(max.map_or(value, |m| m.max(value)));
            }
            match spec.stat {
                StatKind::MeanScaled => {
                    let count = records.len() as i128;
                    if count == 0 {
                        0
                    } else {
                        // Integer division truncates toward zero (documented).
                        i64::try_from(sum / count).unwrap_or_else(|_| {
                            if sum.is_negative() {
                                i64::MIN
                            } else {
                                i64::MAX
                            }
                        })
                    }
                }
                StatKind::MinScaled => min.unwrap_or(0),
                StatKind::MaxScaled => max.unwrap_or(0),
                StatKind::RecordCount => unreachable!("handled in the outer match"),
            }
        }
    };

    if observed_scaled >= spec.lo_scaled && observed_scaled <= spec.hi_scaled {
        CriticVerdict::Valid
    } else {
        CriticVerdict::Invalid {
            reason: CriticReason::StatOutOfBounds {
                stat: spec.stat,
                observed_scaled,
                lo_scaled: spec.lo_scaled,
                hi_scaled: spec.hi_scaled,
            },
        }
    }
}

/// Parse an ASCII decimal scaled integer (optionally signed). Returns `None` on
/// any non-numeric byte, empty field, or overflow — the caller maps that to a
/// deterministic `Unparseable`. No float, no locale, no whitespace trimming
/// (strict so the parse is reproducible).
fn parse_scaled_int(field: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(field).ok()?;
    s.parse::<i64>().ok()
}

fn unparseable(at_offset: u64) -> CriticVerdict {
    CriticVerdict::Invalid {
        reason: CriticReason::Unparseable {
            check: CheckKind::StatBounds,
            at_offset,
        },
    }
}
