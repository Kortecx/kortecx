//! `kx-grammar` — the engine-agnostic constrained-generation spec for RC2
//! grammar-constrained tool-calling.
//!
//! A model running a `ReAct` tool turn must emit a tool call the runtime can
//! decode and authorize. Small OSS models emit them in ~13 fragile formats
//! (the `T-GEMMA-PAREN` class), which the `kx_toolcall` parser recovers with
//! heuristics. This crate removes the fragility at the SOURCE: it derives a
//! grammar from the granted tool set so the model can ONLY emit the canonical
//! tool-call envelope `{"tool_call":{"name":…,"version":…,"args":{…}}}` with a
//! `name`/`version` pair drawn from `warrant.tool_grants`.
//!
//! ## Two dialects, one spec
//! llama.cpp wants **GBNF**; Ollama wants a **JSON Schema**. [`ToolEnvelopeSpec`]
//! is the neutral middle: built once from the granted tools, serialized into the
//! opaque `kx_mote::Grammar.raw` carrier at dispatch, then rendered per engine —
//! [`ToolEnvelopeSpec::to_gbnf`] for llama.cpp, [`ToolEnvelopeSpec::to_ollama_format`]
//! for Ollama.
//!
//! ## Two constrained-output uses, one carrier (`RC4c`)
//! The carrier [`GrammarSpec`] is a tagged enum: a [`ToolEnvelopeSpec`] (RC2
//! tool-calling) OR a [`PermutationSpec`] (`RC4c` listwise rerank). The tool envelope
//! renders to GBNF (llama.cpp, armed lazily) + a JSON schema (Ollama). The permutation
//! renders ONLY to an Ollama whole-response `format`; on llama.cpp it degrades to the
//! fail-closed parser (the char-level GBNF crashes some tokenizers — see
//! [`PermutationSpec`]). The enum is `#[serde(untagged)]` so an existing RC2 carrier
//! raw still decodes as [`GrammarSpec::ToolEnvelope`] (the variants have disjoint
//! required keys — `tools` vs `n`).
//!
//! ## Boundaries (SN-8 / D108.2)
//! - **Accept-side only.** The grammar can only NARROW what the model emits. It
//!   is NOT the authority gate — `kx_toolcall::parse_tool_call` still resolves
//!   the name against `warrant.tool_grants` by exact equality and `validate_args`
//!   still type-checks the args. A grammar bug can never mint an ungranted tool.
//! - **Off-digest.** The spec is derived at dispatch and carried off-digest; it is
//!   never journaled and never participates in `MoteId`. This crate is pure and
//!   FFI-free — it produces strings, it does not touch any engine.

// Inline unit tests use `.unwrap()`/`.expect()` for fixture construction (the
// workspace lints deny these in library code, allow in tests).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod carrier;
mod error;
mod gbnf;
mod ollama;
mod permutation;
mod spec;

pub use carrier::GrammarSpec;
pub use error::GrammarError;
pub use permutation::PermutationSpec;
pub use spec::{ToolEnvelopeSpec, ToolSpec};
