//! # kx-refusal — submission-time refusal predicates (the safety gate)
//!
//! A Mote submission is checked against a closed vocabulary of **refusal
//! predicates** before any dispatch, inference, or commit. This crate is the
//! pure, total, dependency-light home of that gate: the [`SubmissionRefusal`]
//! vocabulary, the [`WorkflowSubmission`] shape, the [`ToolResolution`] outcome,
//! and the validators.
//!
//! ## Two validation surfaces
//!
//! - [`validate_submission`] / [`validate_submission_with_idempotency`] reason
//!   over a **full** [`WorkflowSubmission`] (`BTreeMap<MoteId, Mote>`) — they run
//!   the sibling-dependent predicates (R-2/R-4/R-5/R-6/R-9) that need the whole
//!   critic/producer graph. This is the future SDK / workflow-submission path.
//! - [`validate_mote_submission`] reasons over a **single** Mote at the
//!   coordinator's `SubmitMote` boundary (M1.3) — it runs ONLY the
//!   sibling-INDEPENDENT predicates (R-1, R-7-self, R-8, R-14, R-15) plus the
//!   R-10 / D66 idempotency gate with a **fail-closed** [`ToolResolution`].
//!
//! ## Why a separate leaf crate
//!
//! The refusal vocabulary depends only on `kx-mote`, `kx-tool-registry`, and
//! `kx-warrant`. Holding it here — below `kx-executor` — lets the control-plane
//! `kx-coordinator` enforce refusals at `SubmitMote` WITHOUT pulling
//! `kx-executor`'s inference / llama.cpp stack into the orchestration server.
//! `kx-executor` re-exports this crate's surface for back-compat, so its own
//! lifecycle / commit-protocol modules and tests see the same names.
//!
//! ## SN-8 — model proposes, runtime enforces
//!
//! Every predicate is a **pure, total** function of its inputs — same Mote, same
//! resolution ⇒ same refusal (no clock / host / RNG / float). A refusal is a
//! structural fact about an *unsafe construction*, never a similarity score.

#![forbid(unsafe_code)]

mod refusal;

pub use refusal::{
    native_critic_shape, native_judge_shape, refusal_from_narrowing, validate_mote_submission,
    validate_submission, validate_submission_with_idempotency, SubmissionRefusal, ToolResolution,
    WorkflowSubmission,
};
