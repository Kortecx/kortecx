//! Ollama `format` JSON-Schema rendering of a [`crate::ToolEnvelopeSpec`].
//!
//! Ollama constrains the WHOLE response to a JSON Schema (it has no lazy /
//! triggered mode — see `kx-ollama`'s honest-degrade). The envelope level is
//! always pinned: `tool_call` carries a `name`, a `version` and `args`.
//!
//! **Per-tool argument TYPING reaches this leg too.** It did not used to: `args`
//! was `{"type":"object"}` here while the GBNF leg rendered the declared
//! parameters, so the same warrant constrained llama.cpp and did not constrain
//! Ollama. The accept-side `validate_args` gate refuses a bad call on BOTH
//! engines, so this was never a safety gap — but on Ollama the model had to emit
//! the violation first and the turn was spent finding out. Typed args here mean
//! the model cannot spend a turn that way.
//!
//! The two legs are deliberately at parity on what they do NOT constrain:
//! numeric bounds and length caps stay with `validate_args` on both (a tight
//! digit-range grammar is brittle for weak models — the same D108.2 rationale
//! `gbnf::render_typed_args` records).

use kx_tool_registry::{InputSchema, ParamType};
use serde_json::{json, Map, Value};

use crate::spec::{ToolEnvelopeSpec, ToolSpec};

/// Render the spec to an Ollama `format` JSON Schema.
pub(crate) fn render(spec: &ToolEnvelopeSpec) -> Value {
    if spec.tools.is_empty() {
        // Defensive: caller guards `is_empty`; a generic object is the safest
        // never-broken fallback.
        return json!({ "type": "object" });
    }
    let mut arms = tool_call_arms(spec);
    if arms.len() == 1 {
        // Either the untyped flat envelope, or a lone typed tool — one arm needs
        // no alternation.
        arms.remove(0)
    } else {
        json!({ "oneOf": arms })
    }
}

/// The tool-call arm(s) for this spec.
///
/// When NO tool declares a typed schema this is the single flat envelope the
/// renderer has always produced — one `name` enum over the distinct granted ids,
/// a free `version`, and generic-object `args`. That case must stay
/// byte-identical: it is the case every caller already handled, and it is pinned
/// by `an_untyped_spec_renders_the_flat_envelope_byte_for_byte`.
///
/// When ANY tool is typed the envelope splits into one arm per tool, because a
/// single flat schema cannot say "these args go with THAT name" — exactly the
/// reason the GBNF leg emits one `call{i}` alternative per tool. Splitting also
/// pins `name` to `version` per arm, which the flat form could not: it let a
/// model pair one tool's name with another's version. Untyped tools keep
/// generic-object args in their own arm, so a mixed grant set works.
fn tool_call_arms(spec: &ToolEnvelopeSpec) -> Vec<Value> {
    if spec.tools.iter().all(|t| t.arg_schema.is_none()) {
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
        return vec![envelope_arm(
            &json!({ "type": "string", "enum": names }),
            &json!({ "type": "string" }),
            &json!({ "type": "object" }),
        )];
    }
    spec.tools.iter().map(typed_arm).collect()
}

/// One tool's arm: `name` and `version` pinned to exactly this tool, `args`
/// rendered from its declared schema (generic object when it declares none).
fn typed_arm(tool: &ToolSpec) -> Value {
    envelope_arm(
        // A single-element `enum`, NOT `const`. Both express "exactly this
        // value", but `enum` is already carried through the pinned
        // json-schema→GBNF converter by the name enum above, and `const` is not
        // — this is not the place to find out which keywords it supports.
        &json!({ "type": "string", "enum": [tool.name.clone()] }),
        &json!({ "type": "string", "enum": [tool.version.clone()] }),
        &args_schema(tool.arg_schema.as_ref()),
    )
}

/// The shared `{"tool_call": {name, version, args}}` envelope shape.
fn envelope_arm(name: &Value, version: &Value, args: &Value) -> Value {
    json!({
        "type": "object",
        "properties": {
            "tool_call": {
                "type": "object",
                "properties": { "name": name, "version": version, "args": args },
                "required": ["name", "version", "args"]
            }
        },
        "required": ["tool_call"]
    })
}

/// The `args` sub-schema for a declared [`InputSchema`] — the Ollama counterpart
/// of `gbnf::render_typed_args`. `None` ⇒ the generic object.
///
/// Declared order is preserved (it is the tool's identity contract); JSON Schema
/// is order-tolerant, so unlike the GBNF leg this imposes no key order on the
/// model. `deny_unknown` becomes `additionalProperties: false`, which is what
/// makes the schema closed — and it is also the predicate the gateway uses to
/// decide a schema may be rendered at all.
fn args_schema(schema: Option<&InputSchema>) -> Value {
    let Some(schema) = schema else {
        return json!({ "type": "object" });
    };
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();
    for param in &schema.params {
        properties.insert(param.name.clone(), value_schema(&param.ty));
        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }
    let mut out = json!({ "type": "object", "properties": Value::Object(properties) });
    if !required.is_empty() {
        out["required"] = Value::Array(required);
    }
    if schema.deny_unknown {
        out["additionalProperties"] = json!(false);
    }
    out
}

/// The JSON-Schema value for a declared [`ParamType`]. Mirrors
/// `gbnf::value_fragment` arm for arm, including what it declines to constrain:
/// bounds and length caps are `validate_args`'s job on both engines.
fn value_schema(ty: &ParamType) -> Value {
    match ty {
        ParamType::Bool => json!({ "type": "boolean" }),
        ParamType::Int { .. } => json!({ "type": "integer" }),
        ParamType::Str { .. } | ParamType::Bytes { .. } => json!({ "type": "string" }),
        // An empty allowed-set would render an unsatisfiable `enum: []`, making the
        // whole turn undecodable. The GBNF leg degrades the same way, to `jstring`.
        ParamType::Enum { allowed } if allowed.is_empty() => json!({ "type": "string" }),
        ParamType::Enum { allowed } => {
            json!({ "type": "string", "enum": allowed.iter().cloned().collect::<Vec<_>>() })
        }
    }
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
    // The tool arms are SPLICED beside the answer arm, never nested as a
    // `oneOf` inside this `oneOf`. For an untyped spec that is one arm and the
    // result is byte-identical to before. For a typed spec it keeps the schema
    // one level deep rather than betting on the converter's handling of nested
    // alternation — and the arms stay mutually exclusive either way, since every
    // tool arm requires `tool_call` and the answer arm is a closed object
    // requiring `answer`.
    let mut arms = tool_call_arms(spec);
    arms.push(answer_arm());
    json!({ "oneOf": arms })
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
        // `minLength` is what stops the settle arm being satisfied by NOTHING. An
        // answer-force turn (a duplicate-call rejection, or the near-budget nudge)
        // DROPS the tool arm, so this schema is the model's only legal output — and an
        // unbounded `string` let a weak model emit `{"answer": ""}`, which parses, commits,
        // and settles the chain on an empty answer. Observed live: a model that re-proposed
        // an identical call was refused, forced onto this arm, and settled with nothing,
        // scoring 0 on a task whose tool had already returned the right number. A forced
        // answer that says nothing is strictly worse than a dead-letter, because it looks
        // like the run succeeded.
        "properties": { "answer": { "type": "string", "minLength": 1 } },
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
/// `kx_toolcall::parse_permutation` is the authority on permutation validity.
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

    /// RULE-53 PIN. A spec in which NO tool declares a typed schema must render
    /// BYTE-IDENTICALLY to the pre-typed-args envelope. Typed args were added by
    /// giving each tool its own arm; the danger of that change is not the typed
    /// case (which is new and has its own tests) but the UNTYPED case, which every
    /// caller already handled correctly and which must not move a byte.
    ///
    /// The expected value is written as a LITERAL rather than derived from the
    /// renderer, so a refactor cannot quietly redefine what "identical" means.
    #[test]
    fn an_untyped_spec_renders_the_flat_envelope_byte_for_byte() {
        let spec = ToolEnvelopeSpec::new(vec![
            ToolSpec::new("notion/search", "1"),
            ToolSpec::new("slack/read_channel", "2"),
        ]);
        assert_eq!(
            render(&spec),
            json!({
                "type": "object",
                "properties": {
                    "tool_call": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "enum": ["notion/search", "slack/read_channel"] },
                            "version": { "type": "string" },
                            "args": { "type": "object" }
                        },
                        "required": ["name", "version", "args"]
                    }
                },
                "required": ["tool_call"]
            }),
            "an untyped spec must render the RC2 flat envelope unchanged"
        );
        // …and the union must still be exactly TWO arms: that flat envelope + the answer.
        let union = render_union(&spec);
        let arms = union["oneOf"].as_array().expect("union ⇒ oneOf arms");
        assert_eq!(arms.len(), 2, "an untyped union keeps its two-arm shape");
        assert_eq!(arms[0], render(&spec));
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

    /// The settle arm cannot be satisfied by NOTHING.
    ///
    /// On an answer-force turn the tool arm is dropped, so this schema is the model's
    /// only legal output. With an unbounded `string` a weak model emitted
    /// `{"answer": ""}` — which parses, commits, and settles the chain having said
    /// nothing, scoring 0 on a task whose tool had already returned the right value.
    /// That is worse than a dead-letter: it looks like the run succeeded.
    #[test]
    fn the_answer_arm_refuses_an_empty_answer() {
        let spec = ToolEnvelopeSpec::new(vec![ToolSpec::new("slack/read_channel", "1")]);
        for v in [
            render_answer_only(&spec),
            render_union(&spec)["oneOf"][1].clone(),
        ] {
            assert_eq!(
                v["properties"]["answer"]["minLength"],
                json!(1),
                "the answer arm must require at least one character: {v}"
            );
        }
    }

    #[test]
    fn answer_only_empty_spec_degrades_to_a_generic_object() {
        assert_eq!(
            render_answer_only(&ToolEnvelopeSpec::new(vec![])),
            json!({ "type": "object" })
        );
    }
}
