//! The **schema** check: does the payload conform to a declared [`SchemaTag`]?
//!
//! - `Blob` / `Image` / `Audio` — no structural constraint; always `Valid`.
//! - `Text` — must be valid UTF-8.
//! - `Json` — must be valid UTF-8 AND a well-formed JSON document
//!   (`serde_json` validation only; values are never materialized to floats, so
//!   no NaN/float-ordering touches any path).
//! - `Tensor { dtype, shape }` — byte length must equal `product(shape) *
//!   dtype.byte_width()`.
//! - `Vector { dim }` — byte length must equal `dim * 4` (f32 elements).

use kx_critic_types::{CriticReason, CriticVerdict, SchemaFault, SchemaSpec, SchemaTag};

/// Evaluate the schema check. Total + deterministic.
#[must_use]
pub fn eval(spec: &SchemaSpec, input: &[u8]) -> CriticVerdict {
    match &spec.expected {
        SchemaTag::Blob | SchemaTag::Image | SchemaTag::Audio => CriticVerdict::Valid,
        SchemaTag::Text => match utf8_fault(input) {
            None => CriticVerdict::Valid,
            Some(detail) => invalid(SchemaTag::Text, detail),
        },
        SchemaTag::Json => {
            if let Some(detail) = utf8_fault(input) {
                return invalid(SchemaTag::Json, detail);
            }
            // UTF-8 already established; validate well-formedness without
            // materializing numbers (IgnoredAny visits structure only).
            match serde_json::from_slice::<serde::de::IgnoredAny>(input) {
                Ok(_) => CriticVerdict::Valid,
                Err(e) => invalid(
                    SchemaTag::Json,
                    SchemaFault::NotJson {
                        at_offset: json_error_offset(&e, input.len()),
                    },
                ),
            }
        }
        SchemaTag::Tensor { dtype, shape } => {
            let elems = shape.iter().copied().fold(1u64, u64::saturating_mul);
            let expected_bytes = elems.saturating_mul(dtype.byte_width());
            check_len(
                expected_bytes,
                input.len() as u64,
                elems,
                SchemaTag::Tensor {
                    dtype: *dtype,
                    shape: shape.clone(),
                },
            )
        }
        SchemaTag::Vector { dim } => {
            let elems = u64::from(*dim);
            let expected_bytes = elems.saturating_mul(4); // f32
            check_len(
                expected_bytes,
                input.len() as u64,
                elems,
                SchemaTag::Vector { dim: *dim },
            )
        }
    }
}

fn check_len(
    expected_bytes: u64,
    actual_bytes: u64,
    expected_elems: u64,
    expected: SchemaTag,
) -> CriticVerdict {
    if expected_bytes == actual_bytes {
        CriticVerdict::Valid
    } else {
        invalid(
            expected,
            SchemaFault::ShapeMismatch {
                expected_elems,
                actual_bytes,
            },
        )
    }
}

fn invalid(expected: SchemaTag, detail: SchemaFault) -> CriticVerdict {
    CriticVerdict::Invalid {
        reason: CriticReason::SchemaMismatch { expected, detail },
    }
}

/// Returns `Some(NotUtf8 { at_offset })` if `input` is not valid UTF-8, else
/// `None`. `std::str::from_utf8`'s error reports the first invalid byte offset.
fn utf8_fault(input: &[u8]) -> Option<SchemaFault> {
    match std::str::from_utf8(input) {
        Ok(_) => None,
        Err(e) => Some(SchemaFault::NotUtf8 {
            at_offset: e.valid_up_to() as u64,
        }),
    }
}

/// Best-effort byte offset of a serde_json parse failure. serde_json reports a
/// 1-based column on the failing line; we clamp to the input length so the
/// offset is always in range (the value is diagnostic, not identity-bearing
/// beyond being deterministic for identical input).
fn json_error_offset(e: &serde_json::Error, len: usize) -> u64 {
    let col = e.column();
    let off = col.saturating_sub(1);
    off.min(len) as u64
}
