// SPDX-License-Identifier: Apache-2.0
//! `kx-context-assembler` — deterministic context assembly (D33, first part).
//!
//! Per-Mote, before inference, the executor calls [`assemble`] to resolve the
//! Mote's **explicit dependency closure** (upstream committed `result_ref`s
//! along Data edges + granted tool defs) into actual content bytes. The model
//! reasons over **RESOLVED CONTENT ONLY**. Hashes stay in orchestration and
//! NEVER enter the context window.
//!
//! # The invariant
//!
//! ```text
//! same input hashes  →  byte-identical AssembledContext
//! ```
//!
//! Pure function: no clock, no global state, no I/O outside the explicit
//! interfaces ([`Snapshot`], [`ContentStore`], [`ToolRegistry`]). Recovery
//! re-assembles bit-for-bit; replay determinism flows from this.
//!
//! [`Snapshot`]: kx_projection::Snapshot
//! [`ContentStore`]: kx_content::ContentStore
//! [`ToolRegistry`]: kx_tool_registry::ToolRegistry
//!
//! # Deterministic order
//!
//! Items are emitted in this order:
//!
//! 1. **Parents** along Data edges, sorted by `(MoteId bytes, edge.kind, edge.non_cascade)`.
//!    Control edges contribute no content (they're synchronization, not data).
//! 2. **Tools** resolved via the registry from `warrant.tool_grants`, sorted
//!    by `(tool_id, tool_version)`.
//!
//! Same workflow → same order → same byte stream.
//!
//! # The model NEVER sees a hash
//!
//! Every [`AssembledItem::bytes`] field carries RESOLVED CONTENT (the bytes the
//! content store returned). `source_ref` is internal bookkeeping for replay
//! reproducibility — exposed for journaling but never fed into the model's
//! prompt.
//!
//! # The edge-as-relevance-oracle rule
//!
//! Only the Mote's declared parents contribute context. No history-wide
//! retrieval, no embedding-similarity lookup, no implicit "find related past
//! Motes" path. **Implicit retrieval is forbidden** because it would be
//! non-deterministic on its inputs. If a workflow needs additional context,
//! the author adds a parent Mote that produces it — making the dependency
//! EXPLICIT in the graph.
//!
//! # Context-overflow seam
//!
//! If the assembled closure exceeds `window_bytes`, [`assemble`] returns
//! [`AssemblyError::OverflowDecisionRequired`] with the measured closure size.
//! The caller chooses a deterministic resolution path:
//!
//! - **(a) Fixed deterministic ranking + truncation** — a stable sort key
//!   selects the top-N items that fit. The remaining items are dropped from
//!   this Mote's context; the workflow author can re-add them via explicit
//!   shaping if needed.
//! - **(b) Summarization as its own committed Mote** — a new Mote takes the
//!   overflowing parents as input, calls the model to produce a summary,
//!   commits the summary as its `result_ref`. The original Mote then takes
//!   the SUMMARY's `result_ref` as input.
//!
//! **Forbidden**: letting the model choose at inference time (non-deterministic).
//! Set `window_bytes = usize::MAX` to disable the overflow check (the assembler
//! returns whatever fits).
//!
//! # Reading further
//!
//! - `docs/design/context-assembly.md` (private corpus) — the locked D33 spec.
//! - `docs/design/decisions.md` D33 — interlocking with D29, D30, D32.
//! - `05-progress-tracker.md` SN-8 — *model proposes, runtime enforces*.

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
    clippy::needless_pass_by_value,
    // Test fixtures use short paired names like parent_a / parent_b which
    // clippy flags as "too similar" — they're intentionally paired and
    // reading them in pairs is the point. Allow at crate root for test code.
    clippy::similar_names
)]
// Inline test modules are exempted from the workspace deny on `unwrap_used` /
// `expect_used`. Integration tests under tests/*.rs carry per-file allows.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod assemble;
mod errors;
mod types;

pub use assemble::assemble;
pub use errors::AssemblyError;
pub use types::{AssembledContext, AssembledItem};

#[cfg(test)]
mod tests;
