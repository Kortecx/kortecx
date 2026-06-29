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
//! ## Boundaries (SN-8 / D108.2)
//! - **Accept-side only.** The grammar can only NARROW what the model emits. It
//!   is NOT the authority gate — `kx_toolcall::parse_tool_call` still resolves
//!   the name against `warrant.tool_grants` by exact equality and `validate_args`
//!   still type-checks the args. A grammar bug can never mint an ungranted tool.
//! - **Off-digest.** The spec is derived at dispatch and carried off-digest; it is
//!   never journaled and never participates in `MoteId`. This crate is pure and
//!   FFI-free — it produces strings, it does not touch any engine.

mod error;
mod gbnf;
mod ollama;
mod spec;

pub use error::GrammarError;
pub use spec::{ToolEnvelopeSpec, ToolSpec};
