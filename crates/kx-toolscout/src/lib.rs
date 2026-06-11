//! W1.A5 — advisory tool discovery + [`TaskBundle`](kx_bundle::TaskBundle)
//! lowering (the MCP-intelligence core).
//!
//! **SN-8 boundary (load-bearing — the [`kx_dataset::RetrievalIndex`] note
//! restated).** Everything in this crate that scores, ranks, or fuzzily
//! matches is ADVISORY: it orders candidates for a picker/preview surface and
//! nothing else. No score is ever an input to authority — [`lower_to_workflow_def`]
//! takes a bundle and a warrant, refuses any tool outside `warrant.tool_grants`
//! by EXACT `(name, version)` equality (the same gate as
//! `kx-capability`'s broker precheck and `kx-toolcall`'s decode), and the
//! lowered `WorkflowDef` goes through the **frozen** `kx_workflow::compile` +
//! the normal admission/broker path. Similarity can surface a tool; it can
//! never grant one.
//!
//! Module map (one concern per file): [`fingerprint`](ToolFingerprint) — the
//! multilingual manifest shape; `jw` — the in-crate Jaro-Winkler (pure, no new
//! dependency); [`score`](fingerprint_tolerance_score) — the exact-kw →
//! Jaro-Winkler → embedding-cosine ladder in integer basis points;
//! [`index`](ToolManifestIndex) — manifests + vectors behind
//! `RetrievalIndex`; [`lower`](lower_to_workflow_def) — bundle → `WorkflowDef`
//! → (convenience) the frozen compile.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod fingerprint;
mod jw;
mod lower;
mod score;

mod index;

pub use error::ToolScoutError;
pub use fingerprint::{normalize_keyword, ToolFingerprint, TOOL_FINGERPRINT_SCHEMA_VERSION};
pub use index::{Embedder, ToolManifestIndex};
pub use jw::jaro_winkler;
pub use lower::{compile_bundle, lower_to_workflow_def};
pub use score::{fingerprint_tolerance_score, SCORE_MAX_BP};
