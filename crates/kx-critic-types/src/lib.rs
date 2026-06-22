//! # kx-critic-types — the deterministic-critic vocabulary (P4.2)
//!
//! A **critic** is a Mote that reads an upstream producer's committed output and
//! commits a *verdict*. P4.2 makes the common case — a **deterministic check** —
//! a first-class, declarative, reproducible primitive. This crate is the pure
//! **type surface** of that primitive; the evaluator that actually runs the
//! checks lives in the sibling `kx-critic` crate.
//!
//! ## Why a separate types crate
//!
//! `kx-mote` (the core identity crate the whole workspace depends on) carries a
//! [`CheckSpec`] inside `MoteDef` so a critic's check folds into its `MoteId`
//! (reproducible by construction). `kx-mote` must NOT inherit the evaluator's
//! parser dependencies (`regex` / `serde_json`), so the *types* are split out
//! here. This crate depends only on `serde` / `bincode` / `blake3` / `smallvec`
//! / `thiserror`.
//!
//! It is also deliberately **`kx-dataset`-free**: the schema check validates
//! against a self-contained [`SchemaTag`] mirror of `kx_dataset::ContentSchema`,
//! so this crate sits *below* `kx-dataset` (which itself depends on `kx-mote`)
//! with no dependency cycle.
//!
//! ## SN-8 — model proposes, runtime enforces
//!
//! A [`CriticVerdict`] is a **content-addressed fact**: produced by exact
//! deterministic evaluation and compared downstream by **byte-equality only**
//! (never a similarity score). All evidence carried in [`CriticReason`] is
//! integer-scaled — no floats ever touch the identity / commit / memoization
//! path, preserving the no-float precondition of the canonical bincode hash.
//!
//! ## Determinism contract
//!
//! Every encoding here is a **pure, total** function of its input: the same
//! value yields byte-identical [`CriticVerdict::encode`] output across runs,
//! processes, and machines (no clock / host / PID / RNG / float-NaN ordering).
//! [`CheckSpec::hash_into`] folds a spec into a `blake3::Hasher` with stable u8
//! tags, little-endian integers, and canonical (`BTreeSet`-ordered) set
//! iteration.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::cast_possible_truncation
)]
// `.expect()` on canonical-bincode encode of a type WITHOUT floats and WITHOUT
// non-encodable variants IS infallible. The single site (CriticVerdict::encode)
// carries an inline message naming the precondition; this crate-level allow
// suppresses the workspace `clippy::expect_used = "deny"` policy for that one
// legitimate documented use (mirrors kx-mote's MoteDef::hash treatment).
#![allow(clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod spec;
mod verdict;

#[cfg(test)]
mod tests;

pub use spec::{
    CheckSpec, DedupSpec, LlmJudgeSpec, PiiSpec, RecordFraming, SchemaSpec, SchemaTag,
    StatBoundsSpec, TensorDTypeTag,
};
pub use verdict::{
    canonical_config, CheckKind, CriticReason, CriticVerdict, PiiClass, SchemaFault, StatKind,
    VerdictDecodeError, CRITIC_SCHEMA_VERSION,
};
