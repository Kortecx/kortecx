//! The prompt-template engine is pure, deterministic, and fail-closed; the
//! `render_prompts` bind-time pass is identity-bearing (rendered prompt folds
//! into `MoteId`), idempotent, and atomic (a render failure leaves the
//! `WorkflowDef` byte-unchanged). Rendering before `compile` keeps prompt
//! binding coherent with the free-param `bind_param` path (the D121 inbound path).
//!
//! Integration tests compile as their own crate; own lint exemptions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;

use kx_mote::{ConfigKey, ConfigVal, LogicRef, ModelId, ToolName, PROMPT_KEY};
use kx_workflow::{
    compile, permissive_warrant, render_prompts, transform, CompileError, PromptTemplate,
    WorkflowDef, TEMPLATE_KEY,
};

fn model() -> ModelId {
    ModelId("local".into())
}
fn cap() -> ToolName {
    ToolName("demo".into())
}
fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// A one-step body whose transform carries `template` under `TEMPLATE_KEY`.
fn templated_body(seed: u32, step_logic: u8, template: &str) -> WorkflowDef {
    let mut wf = WorkflowDef::new(seed);
    let mut step = transform(
        LogicRef::from_bytes([step_logic; 32]),
        model(),
        permissive_warrant(model()),
        cap(),
    );
    step.config_subset.insert(
        ConfigKey(TEMPLATE_KEY.to_string()),
        ConfigVal(template.as_bytes().to_vec()),
    );
    wf.add_step(step);
    wf
}

/// The bytes the single compiled mote carries under a `config_subset` key.
fn compiled_config(wf: &WorkflowDef, key: &str) -> Option<Vec<u8>> {
    let out = compile(wf).unwrap();
    out.motes[0]
        .mote
        .def
        .config_subset
        .get(&ConfigKey(key.to_string()))
        .map(|v| v.0.clone())
}

// ── pure engine ─────────────────────────────────────────────────────────────

#[test]
fn parse_render_substitutes_in_source_order() {
    let t = PromptTemplate::parse("summarize {topic} for {audience}").unwrap();
    let out = t
        .render(&params(&[("topic", "outages"), ("audience", "SREs")]))
        .unwrap();
    assert_eq!(out, "summarize outages for SREs");
}

#[test]
fn parse_render_is_pure() {
    let t = PromptTemplate::parse("a {x} b {y} c").unwrap();
    let p = params(&[("x", "1"), ("y", "2")]);
    let first = t.render(&p).unwrap();
    for _ in 0..8 {
        assert_eq!(t.render(&p).unwrap(), first, "render is a pure function");
    }
    assert_eq!(first, "a 1 b 2 c");
}

#[test]
fn render_missing_placeholder_fails_closed() {
    let t = PromptTemplate::parse("hi {name}").unwrap();
    assert_eq!(
        t.render(&params(&[])).unwrap_err(),
        CompileError::MissingPlaceholder {
            name: "name".to_string()
        }
    );
}

#[test]
fn render_unknown_param_fails_closed() {
    let t = PromptTemplate::parse("hi {name}").unwrap();
    let err = t
        .render(&params(&[("name", "x"), ("extra", "y")]))
        .unwrap_err();
    assert_eq!(
        err,
        CompileError::UnknownParam {
            name: "extra".to_string()
        }
    );
}

#[test]
fn parse_malformed_fails_closed() {
    for bad in ["{", "{}", "{a b}", "a}", "{a{b}}", "pre {unterminated"] {
        assert!(
            matches!(
                PromptTemplate::parse(bad),
                Err(CompileError::MalformedTemplate { .. })
            ),
            "template {bad:?} must be rejected"
        );
    }
}

#[test]
fn escapes_render_literal_braces() {
    let t = PromptTemplate::parse("{{x}} and {{{y}}}").unwrap();
    // `{{` → `{`, `}}` → `}`; the inner `{y}` is a real slot.
    assert_eq!(t.render(&params(&[("y", "Z")])).unwrap(), "{x} and {Z}");
}

#[test]
fn no_slots_renders_with_no_params() {
    let t = PromptTemplate::parse("plain text, no slots").unwrap();
    assert_eq!(t.render(&params(&[])).unwrap(), "plain text, no slots");
    assert_eq!(t.slots().count(), 0);
}

// ── render_prompts pass (identity-bearing, idempotent, atomic) ───────────────

#[test]
fn rendered_prompt_is_identity_bearing() {
    let mut a = templated_body(9, 1, "topic: {topic}");
    let mut b = templated_body(9, 1, "topic: {topic}");
    assert_eq!(
        render_prompts(&mut a, &params(&[("topic", "incidents")])).unwrap(),
        1
    );
    assert_eq!(
        render_prompts(&mut b, &params(&[("topic", "outages")])).unwrap(),
        1
    );

    // The rendered prompt landed under PROMPT_KEY; the template slot is gone.
    assert_eq!(
        compiled_config(&a, PROMPT_KEY).unwrap(),
        b"topic: incidents"
    );
    assert!(
        compiled_config(&a, TEMPLATE_KEY).is_none(),
        "template slot removed"
    );

    let id_a = compile(&a).unwrap().motes[0].mote.id;
    let id_b = compile(&b).unwrap().motes[0].mote.id;
    assert_ne!(
        id_a, id_b,
        "distinct rendered prompts → distinct Mote identity"
    );
}

#[test]
fn same_params_yield_same_identity() {
    let mut a = templated_body(9, 1, "q={q}");
    let mut b = templated_body(9, 1, "q={q}");
    render_prompts(&mut a, &params(&[("q", "same")])).unwrap();
    render_prompts(&mut b, &params(&[("q", "same")])).unwrap();
    assert_eq!(
        compile(&a).unwrap().motes[0].mote.id,
        compile(&b).unwrap().motes[0].mote.id,
        "identical rendered prompts → identical identity (reproducible)"
    );
}

#[test]
fn render_prompts_is_atomic_on_failure() {
    // Step 0 has a valid template; step 1's template is malformed.
    let mut wf = WorkflowDef::new(9);
    for (lref, tmpl) in [(1u8, "hi {name}"), (2u8, "{")] {
        let mut step = transform(
            LogicRef::from_bytes([lref; 32]),
            model(),
            permissive_warrant(model()),
            cap(),
        );
        step.config_subset.insert(
            ConfigKey(TEMPLATE_KEY.to_string()),
            ConfigVal(tmpl.as_bytes().to_vec()),
        );
        wf.add_step(step);
    }
    let before: Vec<_> = compile(&wf)
        .unwrap()
        .motes
        .iter()
        .map(|m| m.mote.id)
        .collect();
    let err = render_prompts(&mut wf, &params(&[("name", "x")])).unwrap_err();
    assert!(matches!(
        err,
        CompileError::RenderPromptStep { step: 1, .. }
    ));
    let after: Vec<_> = compile(&wf)
        .unwrap()
        .motes
        .iter()
        .map(|m| m.mote.id)
        .collect();
    assert_eq!(
        before, after,
        "a render failure leaves the WorkflowDef byte-unchanged"
    );
}

#[test]
fn render_prompts_is_idempotent() {
    let mut wf = templated_body(9, 1, "v={v}");
    assert_eq!(render_prompts(&mut wf, &params(&[("v", "1")])).unwrap(), 1);
    let id1 = compile(&wf).unwrap().motes[0].mote.id;
    // Second run: the template slot is gone, so nothing is rendered and identity is unchanged.
    assert_eq!(
        render_prompts(&mut wf, &params(&[("v", "ignored")])).unwrap(),
        0
    );
    let id2 = compile(&wf).unwrap().motes[0].mote.id;
    assert_eq!(id1, id2, "re-running render_prompts is a no-op");
}

#[test]
fn bind_param_then_render_through_to_compile_is_coherent() {
    // A step with BOTH a structural slot ("k") and a prompt template.
    let build = || {
        let mut wf = WorkflowDef::new(9);
        let mut step = transform(
            LogicRef::from_bytes([1; 32]),
            model(),
            permissive_warrant(model()),
            cap(),
        );
        step.config_subset
            .insert(ConfigKey("k".to_string()), ConfigVal(Vec::new()));
        step.config_subset.insert(
            ConfigKey(TEMPLATE_KEY.to_string()),
            ConfigVal(b"q={q}".to_vec()),
        );
        wf.add_step(step);
        wf
    };
    // Same structural binding; the only difference is the rendered prompt.
    let mut a = build();
    let mut b = build();
    assert_eq!(a.bind_param("k", &ConfigVal(b"fixed".to_vec())), 1);
    assert_eq!(b.bind_param("k", &ConfigVal(b"fixed".to_vec())), 1);
    render_prompts(&mut a, &params(&[("q", "alpha")])).unwrap();
    render_prompts(&mut b, &params(&[("q", "beta")])).unwrap();
    assert_ne!(
        compile(&a).unwrap().motes[0].mote.id,
        compile(&b).unwrap().motes[0].mote.id,
        "bind → render → compile is identity-coherent (prompt param flows into identity)"
    );
}
