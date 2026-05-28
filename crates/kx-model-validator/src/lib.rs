#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::match_same_arms
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-model-validator — static bind-time fitness type check
//!
//! Model fitness as a **type check over capability sets**: a task declares
//! [`RequiredCapabilities`] (the signature it expects), a model exposes
//! [`ProvidedCapabilities`] (its actual type), and [`check`] performs structural
//! subtyping — the model binds iff `provided ⊇ required`. Runs at **bind time**
//! (model load / before scheduling a Mote that needs it), never as a mid-execution
//! discovery: find the wrong model at load time, not three Motes into a workflow.
//!
//! Three outcomes ([`ValidatorOutcome`]): `TypeOk` (`provided ⊇ required`, clean
//! bind), `DegradedSubtype` (binds, but a soft capability is missing or emulated —
//! e.g. tool-calling via prompting — recorded so the formatter adapter compensates),
//! and `TypeError` (a REQUIRED capability is missing — refuses to bind, names the gap).
//!
//! **Interface, not quality.** It type-checks the model's *signature*, never its
//! output *behavior* — quality is the workflow / eval / critic layer's job. The
//! honest, smaller claim ("the model HAS the required capabilities; not that it'll be
//! GOOD at them") is the source of its credibility; do not let it drift into a quality
//! oracle. **v1 checks the model's *declared* capabilities** (declarations can lie, so
//! `TypeOk` is precisely "TypeOk-as-declared"; the capability broker P1.8.5 is the
//! runtime backstop that catches false declarations at execution). **v2 (deferred)**
//! verifies each declaration via a cached one-time deterministic probe ("declared" →
//! "verified"). The validator has no notion of a house model — no field, no boost; the
//! type-theoretic framing makes no-favoritism structural (see [`Recommender`]).

mod capabilities;
mod check;
mod outcome;
mod provided;
mod recommender;
mod registry;
mod requirements;

pub use kx_mote::ModelId;

pub use capabilities::{License, LicenseConstraint, Modality, Quantization};
pub use check::check;
pub use outcome::{DegradationReason, MissingCapability, ValidatorOutcome};
pub use provided::{ProvidedCapabilities, Soundness};
pub use recommender::{Candidate, RankingPolicy, Recommender};
pub use registry::{InMemoryModelRegistry, ModelRegistry};
pub use requirements::RequiredCapabilities;

#[cfg(test)]
mod tests;
