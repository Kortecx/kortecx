//! Ollama `format` JSON-Schema rendering of a [`crate::ToolEnvelopeSpec`].
//!
//! Ollama constrains the WHOLE response to a JSON Schema (it has no lazy /
//! triggered mode — see `kx-ollama`'s honest-degrade). RC2 renders the envelope
//! level: the `name` is an enum over the granted tool ids, `version`/`args` are
//! structurally pinned. Per-tool argument TYPING is carried in the spec for the
//! GBNF (llama.cpp) leg; Ollama keeps generic-object args in RC2 (the accept-side
//! `validate_args` gate enforces the types identically on both engines).

use serde_json::{json, Value};

use crate::spec::ToolEnvelopeSpec;

/// Render the spec to an Ollama `format` JSON Schema.
pub(crate) fn render(spec: &ToolEnvelopeSpec) -> Value {
    if spec.tools.is_empty() {
        // Defensive: caller guards `is_empty`; a generic object is the safest
        // never-broken fallback.
        return json!({ "type": "object" });
    }

    // Distinct granted names, in the spec's canonical order (sorted by
    // (name, version), so equal names are adjacent — dedup keeps order).
    let mut names: Vec<Value> = Vec::with_capacity(spec.tools.len());
    let mut last: Option<&str> = None;
    for tool in &spec.tools {
        if last != Some(tool.name.as_str()) {
            names.push(Value::String(tool.name.clone()));
            last = Some(tool.name.as_str());
        }
    }

    json!({
        "type": "object",
        "properties": {
            "tool_call": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "enum": names },
                    "version": { "type": "string" },
                    "args": { "type": "object" }
                },
                "required": ["name", "version", "args"]
            }
        },
        "required": ["tool_call"]
    })
}

/// Render an Ollama `format` JSON Schema (RC4c) for a listwise-rerank turn: the
/// WHOLE response is an integer array of length `n` with each item in `[0, n)`.
///
/// Unlike RC2's tool-call envelope (which can appear mid-prose, so Ollama's
/// whole-response `format` honestly degrades — `T-OLLAMA-GRAMMAR-FORMAT`), a rerank
/// turn's ENTIRE output is the permutation, so a strict whole-response schema is
/// exactly right here. `uniqueItems`/range are advisory — the fail-closed
/// `kx_toolcall::parse_permutation` is the authority on permutation validity (SN-8).
pub(crate) fn render_permutation(n: u32) -> Value {
    let max = n.saturating_sub(1);
    json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 0, "maximum": max },
        "minItems": n,
        "maxItems": n,
        "uniqueItems": true
    })
}
