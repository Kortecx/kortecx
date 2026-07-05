//! [`ToolEnvelopeSpec`] — the neutral, engine-agnostic constraint spec.

use kx_tool_registry::InputSchema;
use serde::{Deserialize, Serialize};

use crate::error::GrammarError;

/// One granted tool the model may call this turn: the exact `name`/`version` the
/// envelope must carry, plus an optional typed arg schema.
///
/// `name` is the tool's full id (e.g. `mcp-echo/echo`) — the most specific form,
/// which resolves UNIQUELY through `kx_toolcall`'s `id_matches`. `version` is the
/// granted version (authoritative; the parser ignores the envelope's version for
/// resolution but the grammar pins it so the committed envelope is an exact member
/// of `warrant.tool_grants`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSpec {
    /// The full tool id the model must emit as `name`.
    pub name: String,
    /// The granted version the model must emit as `version`.
    pub version: String,
    /// The tool's typed arg schema. `None` ⇒ args are constrained to a generic
    /// JSON object (the RC2 envelope-first default); `Some` ⇒ args are constrained
    /// to the declared parameters (the per-tool arg-schema STRETCH).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg_schema: Option<InputSchema>,
}

impl ToolSpec {
    /// A tool spec with generic-object args (envelope-first).
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            arg_schema: None,
        }
    }

    /// A tool spec whose args are constrained to `schema` (the stretch).
    #[must_use]
    pub fn with_schema(
        name: impl Into<String>,
        version: impl Into<String>,
        schema: InputSchema,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            arg_schema: Some(schema),
        }
    }
}

/// The set of tools a tool-eligible `ReAct` turn may call — the engine-agnostic
/// constraint. Built once at dispatch from `warrant.tool_grants`, serialized into
/// `kx_mote::Grammar.raw`, then rendered per engine via [`Self::to_gbnf`] /
/// [`Self::to_ollama_format`].
///
/// The tool order is CANONICAL (sorted by `(name, version)`) so the rendered
/// grammar is deterministic across calls with the same grant set — a property the
/// memoizer + replay rely on (same warrant ⇒ byte-identical grammar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolEnvelopeSpec {
    /// The granted tools, in canonical `(name, version)` order.
    pub tools: Vec<ToolSpec>,
    /// RC4c-2c (`T-OLLAMA-GRAMMAR-FORMAT`): opt-in **tool-required** mode. When `true`, the
    /// Ollama backend applies this envelope as a STRICT whole-response `format` — the model
    /// MUST emit a tool call and can no longer answer with prose on this turn. Default
    /// `false` ⇒ honest-degrade to the fail-closed parser (the free-form ANSWER path is
    /// preserved). llama.cpp is unaffected (it already arms a LAZY/triggered GBNF that lets
    /// prose flow until the tool-call opener). Serialized ONLY when set, so the default
    /// carrier stays byte-identical to pre-RC4c-2c (off-digest either way, D108.2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub strict: bool,
    /// `T-RUNAPP-RAG-RECIPE-ROUTE` (gemma3 connector-tool-fire): opt-in **non-strict
    /// UNION** mode. When `true`, the Ollama backend applies this envelope as a
    /// `{"tool_call":{…}} oneOf {"answer":"…"}` whole-response `format` — the Ollama
    /// analog of llama.cpp's LAZY/triggered GBNF: the model is forced to emit PARSEABLE
    /// JSON (a well-formed tool call → it fires, OR a well-formed answer → it settles),
    /// so a free-form gemma3 turn can no longer emit a malformed body that dead-letters.
    /// Unlike `strict` (which forbids answering), the answer arm PRESERVES the settle
    /// path. Mutually exclusive with `strict` (the caller sets at most one). llama.cpp is
    /// unaffected (it already arms a lazy GBNF that lets prose flow until the opener).
    /// Serialized ONLY when set, so the default carrier stays byte-identical (off-digest,
    /// D108.2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub answerable: bool,
    /// `T-GEMMA3-TOOL-LOOP-ANSWER-FORCE` (loop-completeness follow-up to `answerable`):
    /// opt-in **answer-only** mode. When `true`, the Ollama backend applies this envelope
    /// as an `{"answer":"…"}`-ONLY whole-response `format` — the union with the `tool_call`
    /// arm DROPPED — so a weak model (e.g. gemma3) is FORCED to settle instead of re-firing
    /// a duplicate tool call or looping past its budget. Armed by the gateway ONLY on a react
    /// turn whose frozen instruction is a duplicate-rejection re-prompt or the near-budget
    /// settle-nudge (llama.cpp already completes the loop, so its GBNF ignores this flag).
    /// **Mutually exclusive** with `strict`/`answerable` — the caller sets at most one; the
    /// dispatch site gates it on `answerable` (i.e. `!strict`) and clears `answerable` when it
    /// sets this. Serialized ONLY when set, so the default carrier stays byte-identical
    /// (off-digest, D108.2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub answer_only: bool,
}

/// `skip_serializing_if` predicate — omit `strict` when `false` (byte-identical default carrier).
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

impl ToolEnvelopeSpec {
    /// Build a spec from a granted tool set, sorting into canonical order. An
    /// empty set yields an empty spec (the caller MUST NOT arm a grammar for a
    /// turn with no granted tools — `is_empty` guards that at the call site).
    #[must_use]
    pub fn new(mut tools: Vec<ToolSpec>) -> Self {
        tools.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        tools.dedup();
        Self {
            tools,
            strict: false,
            answerable: false,
            answer_only: false,
        }
    }

    /// Set the RC4c-2c opt-in tool-required (`strict`) mode (chainable). See the field docs.
    #[must_use]
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Set the non-strict UNION (`answerable`) mode (chainable). See the field docs.
    #[must_use]
    pub fn with_answerable(mut self, answerable: bool) -> Self {
        self.answerable = answerable;
        self
    }

    /// Set the answer-only (`answer_only`) mode (chainable). See the field docs. The
    /// dispatch site is responsible for the mutual exclusivity (clearing `answerable`
    /// when it sets this); this setter does not enforce it so the builder stays orthogonal.
    #[must_use]
    pub fn with_answer_only(mut self, answer_only: bool) -> Self {
        self.answer_only = answer_only;
        self
    }

    /// True when no tools are granted — a grammar must not be derived.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Serialize into the opaque `kx_mote::Grammar.raw` carrier (canonical JSON).
    ///
    /// # Errors
    /// [`GrammarError::Malformed`] only if serialization fails (not reachable for
    /// the closed spec types, but surfaced rather than panicking).
    pub fn to_raw(&self) -> Result<String, GrammarError> {
        serde_json::to_string(self).map_err(|e| GrammarError::Malformed {
            diagnostic: e.to_string(),
        })
    }

    /// Recover a spec from the opaque `kx_mote::Grammar.raw` carrier.
    ///
    /// # Errors
    /// [`GrammarError::Malformed`] if `raw` is not a serialized spec — the engine
    /// leg must fail the dispatch CLOSED on this (never silently unconstrain).
    pub fn from_raw(raw: &str) -> Result<Self, GrammarError> {
        serde_json::from_str(raw).map_err(|e| GrammarError::Malformed {
            diagnostic: e.to_string(),
        })
    }

    /// Render this spec to a GBNF grammar (llama.cpp dialect).
    #[must_use]
    pub fn to_gbnf(&self) -> String {
        crate::gbnf::render(self)
    }

    /// Render this spec to a JSON Schema (Ollama `format` dialect).
    #[must_use]
    pub fn to_ollama_format(&self) -> serde_json::Value {
        crate::ollama::render(self)
    }

    /// Render this spec to a non-strict UNION JSON Schema (Ollama `format` dialect):
    /// `{"tool_call":{…}} oneOf {"answer":"…"}` — forces PARSEABLE output while keeping
    /// the settle (answer) path open. See [`Self::answerable`].
    #[must_use]
    pub fn to_ollama_union_format(&self) -> serde_json::Value {
        crate::ollama::render_union(self)
    }

    /// Render this spec to an ANSWER-ONLY JSON Schema (Ollama `format` dialect):
    /// `{"answer":"…"}` with the `tool_call` arm DROPPED — forces the model to settle.
    /// See [`Self::answer_only`].
    #[must_use]
    pub fn to_ollama_answer_only_format(&self) -> serde_json::Value {
        crate::ollama::render_answer_only(self)
    }
}
