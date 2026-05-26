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
//! Model fitness as a **type check over capability sets**.
//!
//! - A task declares a [`RequiredCapabilities`] — the type signature it expects.
//! - A model exposes a [`ProvidedCapabilities`] — its actual type.
//! - [`check`] performs structural subtyping: the model is a valid binding iff
//!   its provided capabilities satisfy (are a superset of) the task's required
//!   ones.
//!
//! Runs at **bind time** — when a model is loaded, or before a Mote needing
//! that model is scheduled. It is a **static** check, never a mid-execution
//! discovery. The whole value: find the wrong model at load time, not three
//! Motes into a workflow.
//!
//! ## Three outcomes ([`ValidatorOutcome`])
//!
//! - [`ValidatorOutcome::TypeOk`] — `provided ⊇ required` — clean bind.
//! - [`ValidatorOutcome::DegradedSubtype`] — binds, but an optional/soft
//!   capability is missing or emulated (e.g. tool-calling done via prompting
//!   instead of native). Records the degraded mode for downstream callers
//!   (the formatter adapter compensates).
//! - [`ValidatorOutcome::TypeError`] — a REQUIRED capability is missing —
//!   refuses to bind, names the missing member and why.
//!
//! ## Hard boundary: interface, NOT quality
//!
//! The validator type-checks the model's **interface (signature)**, never its
//! output **quality (behavior)**. Capability is statically checkable; quality
//! is not — that's the workflow / eval / critic layer's job (orchestration vs
//! semantic correctness).
//!
//! The validator's smaller, honest claim ("I prove your model HAS the required
//! capabilities; I do not claim it'll be GOOD at them") is the source of its
//! credibility. Do not let it drift into a quality oracle.
//!
//! ## Soundness boundary: v1 → v2
//!
//! - **v1 (this crate)** — the validator type-checks against the model's
//!   **declared** [`ProvidedCapabilities`]. Declarations can lie. v1's guarantee
//!   is *"the model CLAIMS to satisfy the signature."* All v1 language uses
//!   "declared." `TypeOk` is more precisely "TypeOk-as-declared."
//! - **v2 (deferred)** — a capability probe will verify each declaration via a
//!   one-time deterministic test, result cached. v2 language will use "verified"
//!   and "guaranteed."
//! - In v1, the capability broker (P1.8.5) remains the runtime backstop that
//!   catches false declarations at execution.
//!
//! ## House model competes on equal merit
//!
//! The validator does **not** know which model is the house model. There is
//! no field, no flag, no boost. The type-theoretic framing makes the
//! no-favoritism property structural rather than aspirational. See
//! [`Recommender`] for the ranking discipline.

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
