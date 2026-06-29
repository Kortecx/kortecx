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
}

impl ToolEnvelopeSpec {
    /// Build a spec from a granted tool set, sorting into canonical order. An
    /// empty set yields an empty spec (the caller MUST NOT arm a grammar for a
    /// turn with no granted tools — `is_empty` guards that at the call site).
    #[must_use]
    pub fn new(mut tools: Vec<ToolSpec>) -> Self {
        tools.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
        tools.dedup();
        Self { tools }
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
}
