//! # kx-critic — the deterministic-critic evaluator (P4.2)
//!
//! [`evaluate`] runs one declarative [`CheckSpec`] against a producer's committed
//! output bytes and returns a [`CriticVerdict`]. The four checks the corpus calls
//! out are implemented here:
//!
//! - **schema** ([`checks::schema`]) — does the payload conform to a
//!   [`SchemaTag`] (UTF-8 / JSON well-formedness / tensor·vector byte-length)?
//! - **dedup** ([`checks::dedup`]) — are there duplicate records under a declared
//!   framing + key?
//! - **stat-bounds** ([`checks::stat_bounds`]) — is an aggregate within an
//!   inclusive integer bound?
//! - **PII-leakage** ([`checks::pii`]) — does a forbidden detector class match?
//!
//! ## Contract (load-bearing)
//!
//! [`evaluate`] is **pure, total, and deterministic**:
//! - *Total* — it returns a [`CriticVerdict`] for EVERY input (adversarial,
//!   truncated, empty, non-UTF-8, gigabytes). It never panics; a parse failure is
//!   a deterministic [`CriticReason::Unparseable`], not a crash.
//! - *Pure / deterministic* — the same `(spec, input)` yields a byte-identical
//!   verdict on every run, process, and machine: no clock, host, PID, RNG, or
//!   float-NaN ordering participates. (Floats never appear in a verdict — all
//!   evidence is integer-scaled — so the canonical verdict bytes are stable.)
//! - *Bounded* — O(input) work and O(input) peak memory.
//!
//! ## SN-8
//!
//! A verdict is a content-addressed FACT. The runtime commits
//! `verdict.encode()` to the content store and the projection compares verdicts
//! by byte-equality only (`Valid` vs `Invalid`) — never a similarity score.
//!
//! [`CheckSpec`]: kx_critic_types::CheckSpec
//! [`CriticVerdict`]: kx_critic_types::CriticVerdict
//! [`SchemaTag`]: kx_critic_types::SchemaTag
//! [`CriticReason::Unparseable`]: kx_critic_types::CriticReason::Unparseable

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod checks;
mod convert;
mod evaluate;
mod framing;

#[cfg(test)]
mod tests;

pub use convert::schema_tag_of;
pub use evaluate::evaluate;

// Re-export the full vocabulary so downstream crates depend on `kx-critic` alone.
pub use kx_critic_types::{
    canonical_config, CheckKind, CheckSpec, CriticReason, CriticVerdict, DedupSpec, LlmJudgeSpec,
    PiiClass, PiiSpec, RecordFraming, SchemaFault, SchemaSpec, SchemaTag, StatBoundsSpec, StatKind,
    TensorDTypeTag, VerdictDecodeError, CRITIC_SCHEMA_VERSION,
};
