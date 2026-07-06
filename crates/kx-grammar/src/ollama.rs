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

/// Render a NON-STRICT UNION Ollama `format` JSON Schema: a well-formed tool-call
/// envelope `oneOf` a well-formed answer object — the Ollama analog of llama.cpp's
/// LAZY/triggered GBNF. Forces the whole response to be PARSEABLE JSON (a tool call →
/// it fires, OR an `{"answer":"…"}` object → it settles) WITHOUT forcing tool-required,
/// so a free-form gemma3 turn can no longer emit a malformed body that dead-letters, yet
/// can still answer. The two arms are disjoint by required key (`tool_call` vs `answer`,
/// `additionalProperties:false` on the answer arm) so `oneOf` matches exactly one arm.
///
/// (`oneOf` is honored by the pinned Ollama/llama.cpp json-schema→GBNF converter —
/// verified live against gemma3:12b: a tool-eliciting turn emits the exact envelope, a
/// non-tool turn emits `{"answer":…}`.)
pub(crate) fn render_union(spec: &ToolEnvelopeSpec) -> Value {
    if spec.tools.is_empty() {
        // Defensive: caller guards `is_empty`; a generic object is the safest
        // never-broken fallback (mirrors `render`).
        return json!({ "type": "object" });
    }
    json!({
        "oneOf": [
            render(spec),
            answer_arm()
        ]
    })
}

/// Render an ANSWER-ONLY Ollama `format` JSON Schema: the closed `{"answer":"…"}`
/// object with NO `tool_call` arm — the union's answer arm ALONE. This FORCES a weak
/// model (e.g. gemma3) to settle on a parseable answer instead of re-firing a
/// duplicate tool call or looping past its budget (`T-GEMMA3-TOOL-LOOP-ANSWER-FORCE`,
/// the loop-completeness follow-up to `render_union`). Armed by the gateway ONLY on a
/// react turn whose (frozen) instruction is a duplicate-rejection re-prompt or the
/// near-budget settle-nudge; llama.cpp is unaffected (its GBNF ignores the flag and
/// already completes the loop). Same empty-guard as `render`/`render_union`.
pub(crate) fn render_answer_only(spec: &ToolEnvelopeSpec) -> Value {
    if spec.tools.is_empty() {
        // Defensive: caller guards `is_empty`; a generic object is the safest
        // never-broken fallback (mirrors `render`/`render_union`).
        return json!({ "type": "object" });
    }
    answer_arm()
}

/// The closed `{"answer":<string>}` schema — arm 1 of the union AND the WHOLE of the
/// answer-only format. Shared by [`render_union`] + [`render_answer_only`] so the two
/// renderers cannot drift.
fn answer_arm() -> Value {
    json!({
        "type": "object",
        "properties": { "answer": { "type": "string" } },
        "required": ["answer"],
        "additionalProperties": false
    })
}

/// Render an Ollama `format` JSON Schema (`RC4c`) for a listwise-rerank turn: the
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ToolSpec;

    #[test]
    fn union_has_a_toolcall_arm_and_an_answer_arm() {
        let spec = ToolEnvelopeSpec::new(vec![
            ToolSpec::new("slack/read_channel", "1"),
            ToolSpec::new("notion/search", "1"),
        ]);
        let v = render_union(&spec);
        let arms = v["oneOf"].as_array().expect("union ⇒ oneOf arms");
        assert_eq!(arms.len(), 2, "exactly a tool_call arm + an answer arm");
        // Arm 0 is the EXACT tool-call envelope schema (name enum over the granted ids).
        assert_eq!(arms[0], render(&spec));
        let names = arms[0]["properties"]["tool_call"]["properties"]["name"]["enum"]
            .as_array()
            .expect("name enum");
        assert!(names.iter().any(|n| n == "slack/read_channel"));
        assert!(names.iter().any(|n| n == "notion/search"));
        // Arm 1 is a closed `{"answer":<string>}` object (disjoint from the tool_call arm).
        assert_eq!(arms[1]["type"], "object");
        assert_eq!(arms[1]["properties"]["answer"]["type"], "string");
        assert_eq!(arms[1]["required"], json!(["answer"]));
        assert_eq!(arms[1]["additionalProperties"], json!(false));
    }

    #[test]
    fn empty_spec_degrades_to_a_generic_object() {
        // Defensive: the caller guards `is_empty`, but a never-broken fallback must hold.
        assert_eq!(
            render_union(&ToolEnvelopeSpec::new(vec![])),
            json!({ "type": "object" })
        );
    }

    #[test]
    fn answer_only_is_the_closed_answer_arm_alone() {
        let spec = ToolEnvelopeSpec::new(vec![ToolSpec::new("slack/read_channel", "1")]);
        let v = render_answer_only(&spec);
        // NO tool_call arm — the model is forced to settle, not fire.
        assert!(v.get("oneOf").is_none(), "answer-only has no union arms");
        assert!(
            v["properties"].get("tool_call").is_none(),
            "answer-only must NOT expose a tool_call"
        );
        assert_eq!(v["type"], "object");
        assert_eq!(v["properties"]["answer"]["type"], "string");
        assert_eq!(v["required"], json!(["answer"]));
        assert_eq!(v["additionalProperties"], json!(false));
        // It is BYTE-identical to the union's answer arm (the shared `answer_arm`).
        let union = render_union(&spec);
        assert_eq!(v, union["oneOf"][1]);
    }

    #[test]
    fn answer_only_empty_spec_degrades_to_a_generic_object() {
        assert_eq!(
            render_answer_only(&ToolEnvelopeSpec::new(vec![])),
            json!({ "type": "object" })
        );
    }
}
