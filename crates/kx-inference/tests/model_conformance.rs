//! **The model-onboarding gate.** Can this runtime drive THIS model agentically?
//!
//! Onboarding a new model used to mean reading its chat template by hand, adding a table
//! entry, and then finding out from a benchmark days later whether tool calling actually
//! worked. The expensive part was never the table entry — it was not knowing. This answers
//! it in minutes, for any GGUF, with no code change:
//!
//! ```text
//! just model-conformance ~/.kx-models/<model>.gguf
//! ```
//!
//! Five stages, each of which can fail loudly and separately:
//!
//! | stage | question |
//! |---|---|
//! | `dialect` | does the model DECLARE how it spells a tool call, and did we read it? |
//! | `engagement` | does the sampler actually ENGAGE on what the model emits? |
//! | `prose-mask` | does it stay off ordinary prose? |
//! | `tool-call` | are the emitted arguments well-formed and grant-legal? |
//! | `agentic` | (driven by the wrapper script) does a real multi-step task complete? |
//!
//! **Why `engagement` and `prose-mask` are separate stages, and why both must pass.** A
//! trigger that never fires leaves tool arguments completely unconstrained — the defect
//! this gate was built for, which stayed invisible for eight sessions because the tolerant
//! parser recovered the call either way and every "did the tool fire?" check stayed green.
//! A trigger that fires too EAGERLY is worse: it masks an ordinary prose answer as a tool
//! call. Reporting one without the other would be reporting half a result.
//!
//! **This test FAILS rather than skips when it has no model.** A skip is indistinguishable
//! from a pass, and an onboarding gate that silently passes is worse than none.

#![cfg(feature = "llamacpp")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_grammar::{GrammarSpec, ToolEnvelopeSpec, ToolSpec};
use kx_inference::{
    Grammar, InferenceBackend, InferenceInput, InferenceParams, LlamaInferenceBackend,
};
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    WarrantSpec,
};

/// The env var naming the GGUF under test.
const MODEL_ENV: &str = "KX_CONFORMANCE_GGUF";

/// The two tools the probe grants. Distinct names with no shared segment, so neither is
/// ambiguous and both are legal in a native (version-less) branch.
const TOOLS: [(&str, &str); 2] = [("mcp-calc/calc", "1"), ("mcp-kv/get", "1")];

fn model_path() -> PathBuf {
    let raw = std::env::var(MODEL_ENV).unwrap_or_else(|_| {
        panic!(
            "PRECONDITION: {MODEL_ENV} is unset. This is the model-onboarding GATE — it \
             fails instead of skipping, because a skip reads exactly like a pass and an \
             onboarding gate that silently passes is worse than no gate.\n\
             Run it as: just model-conformance <path-to.gguf>"
        )
    });
    let p = PathBuf::from(&raw);
    assert!(p.is_file(), "{MODEL_ENV}={raw} is not a file");
    p
}

fn backend() -> (LlamaInferenceBackend, ModelId) {
    let path = model_path();
    let id = ModelId("conformance".into());
    (LlamaInferenceBackend::with_model(id.clone(), path), id)
}

/// A warrant granting both probe tools, with budget for a real decode.
fn warrant(model_id: &ModelId) -> WarrantSpec {
    let tool_grants: BTreeSet<ToolGrant> = TOOLS
        .iter()
        .map(|(n, v)| ToolGrant {
            tool_id: ToolName((*n).to_string()),
            tool_version: ToolVersion((*v).to_string()),
        })
        .collect();
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef([0u8; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: model_id.clone(),
            max_input_tokens: 4096,
            max_output_tokens: 256,
            max_calls: 100,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 60_000,
            mem_bytes: 1 << 34,
            wall_clock_ms: 300_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// The declared parameter schema for the calculator probe.
///
/// ⚠ TYPED, not untyped, and that is the whole point. A generic-object args rule is
/// satisfied by `{}` — measured: Gemma-4 wants to write its native `{a:6,b:7,op:mul}`,
/// and when the grammar demands JSON it takes the cheapest legal exit and emits an EMPTY
/// object. Valid, and useless. Constraining arguments only buys something when the
/// constraint carries the tool's DECLARED parameters, which is what production does
/// whenever a tool's schema sets `deny_unknown`.
fn calc_schema() -> kx_tool_registry::InputSchema {
    use kx_tool_registry::{InputSchema, ParamSpec, ParamType};
    InputSchema {
        params: vec![
            ParamSpec {
                name: "a".into(),
                ty: ParamType::Int {
                    min: None,
                    max: None,
                },
                required: true,
            },
            ParamSpec {
                name: "b".into(),
                ty: ParamType::Int {
                    min: None,
                    max: None,
                },
                required: true,
            },
            ParamSpec {
                name: "op".into(),
                ty: ParamType::Enum {
                    allowed: ["add", "div", "mul", "sub"]
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect(),
                },
                required: true,
            },
        ],
        deny_unknown: true,
    }
}

/// Decoding params with the tool grammar armed (the constrained arm).
fn constrained_params() -> InferenceParams {
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::with_schema(TOOLS[0].0, TOOLS[0].1, calc_schema()),
        ToolSpec::new(TOOLS[1].0, TOOLS[1].1),
    ]);
    let raw = GrammarSpec::ToolEnvelope(spec)
        .to_raw()
        .expect("serialize the grammar carrier");
    InferenceParams {
        max_output_tokens: 256,
        temperature_bps: 0,
        grammar: Some(Grammar { raw }),
        ..Default::default()
    }
}

/// The same params with NO grammar — the control arm, one variable apart.
fn control_params() -> InferenceParams {
    InferenceParams {
        grammar: None,
        ..constrained_params()
    }
}

/// The tool menu, as a system message.
///
/// Override with `KX_CONFORMANCE_SYSTEM` when onboarding a model that expects its tool
/// menu in a particular shape. This is not a workaround, it is the knob onboarding needs:
/// a chat template renders the tool PROTOCOL only when the caller can pass it a `tools`
/// list, and `llama_chat_apply_template` accepts role/content pairs only. Measured:
/// Qwen2.5 answers this prose menu with bare text and never emits its own `<tool_call>`
/// marker, but given the Hermes menu its own template would have rendered, it emits a
/// perfectly-formed call on the FIRST token. Same model, same dialect, same grammar — the
/// only variable is how the menu was presented.
fn tool_system() -> String {
    std::env::var("KX_CONFORMANCE_SYSTEM").unwrap_or_else(|_| {
        "You can call tools. Available: mcp-calc/calc (args: a, b, op where op is one of \
         add, div, mul, sub) and mcp-kv/get (args: key). When a tool is needed, reply with \
         exactly one tool call and nothing else."
            .to_string()
    })
}

/// A tool-eligible instruction. Worded so the natural verb (`multiply`) is OUTSIDE the
/// tool's declared vocabulary, which is what makes a violation visible at all.
const TOOL_TASK: &str = "What is 6 multiplied by 7? You must use the calculator tool to \
work it out.";

/// Render through the MODEL'S OWN chat template, the way production does.
///
/// ⚠ Not a detail. Dispatching the raw string instead measures the model outside the turn
/// framing it was trained on: measured, Qwen2.5 answered a raw prompt with bare prose
/// (`mcp-calc/calc a 6 b 7 op mul`) and never emitted its own `<tool_call>` marker at all,
/// which would have been reported as a dialect failure when it was a HARNESS failure.
fn render(backend: &LlamaInferenceBackend, id: &ModelId, system: &str, user: &str) -> String {
    backend
        .render_chat(id, system, user)
        .unwrap_or_else(|| format!("{system}\n\n{user}"))
}

/// Prose probes. The second and third are ADVERSARIAL: they discuss tool calling in
/// ordinary English, including the marker words, and must still come back as prose.
const PROSE_PROBES: [&str; 3] = [
    "In two sentences, why does writing a checklist before a deploy reduce mistakes?",
    "Explain in prose what a tool_call envelope is and when a model would emit one.",
    "Describe the difference between 3 < 5 and 7 > 2 in plain words.",
];

#[test]
#[ignore = "the model-onboarding gate: loads a real GGUF and decodes. Run via `just model-conformance <gguf>`"]
fn model_conformance_gate() {
    let (backend, id) = backend();
    let w = warrant(&id);
    let mut failures: Vec<String> = Vec::new();

    // ---- stage 1: dialect ---------------------------------------------------------
    let dialects = backend
        .model_dialects(&id)
        .expect("reading the model's dialects must not fail");
    match &dialects.derived {
        Some(d) => println!(
            "dialect     DECLARED by the model    open {:?} close {:?} marker {:?} shape {:?}   OK",
            d.open, d.close, d.call_marker, d.shape
        ),
        None => println!(
            "dialect     not declared — falling back to {} known dialects   WARN\n\
             \x20           (the model's chat template never mentions tool_calls, so the \
             runtime arms every dialect it knows rather than the one this model uses)",
            dialects.armed.len()
        ),
    }
    println!(
        "            armed: {}",
        dialects
            .armed
            .iter()
            .map(|d| d.id.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // ---- stage 2: engagement ------------------------------------------------------
    let before = backend.grammar_engagement();
    let constrained = backend
        .dispatch(
            &id,
            &InferenceInput::Text(render(&backend, &id, &tool_system(), TOOL_TASK)),
            &constrained_params(),
            &w,
        )
        .expect("the constrained dispatch must complete");
    let e = backend.grammar_engagement().since(before);
    let constrained_text = String::from_utf8_lossy(&constrained.bytes).to_string();

    if e.armed == 0 {
        failures.push(
            "engagement: NO grammar was armed at all. The tool grammar was never derived \
             for this turn — a grant/dispatch problem, not a dialect one."
                .to_string(),
        );
    } else if e.looks_uninstrumented() {
        failures.push(format!(
            "engagement: INSTRUMENT BROKEN — {} stage(s) armed but llama.cpp reported \
             neither an engagement nor a single awaiting token. The log sink is not \
             observing this sampler, so a zero here proves nothing about the model.",
            e.armed
        ));
    } else if e.engaged == 0 {
        // Two very different defects wear the same zero, and they send a reader to
        // different code. Ask the shared parser whether the model even TRIED.
        let attempted =
            kx_toolcall::looks_like_an_attempted_tool_call(constrained_text.as_bytes(), &w)
                || dialects
                    .armed
                    .iter()
                    .filter_map(|d| d.grammar_prefix())
                    .any(|open| constrained_text.contains(open));
        if attempted {
            failures.push(format!(
                "engagement: the model ATTEMPTED a tool call in a syntax no armed trigger \
                 matches (armed {} / engaged 0 / awaiting {}), so its arguments were \
                 generated completely UNCONSTRAINED. This is a DIALECT gap — the model \
                 spells calls in a form neither its template declared nor the fallback set \
                 covers. Emitted: {constrained_text:?}",
                e.armed, e.awaiting
            ));
        } else {
            failures.push(format!(
                "engagement: the model did NOT ATTEMPT a tool call at all (armed {} / \
                 engaged 0 / awaiting {}). This is NOT a dialect or trigger defect — \
                 nothing was there to trigger on. The usual cause is the TOOL MENU: a chat \
                 template renders its tool-call protocol only when the caller can pass a \
                 `tools` list, and the runtime presents the menu as prose instead. Re-run \
                 with KX_CONFORMANCE_SYSTEM set to the menu this model's template expects. \
                 Emitted: {constrained_text:?}",
                e.armed, e.awaiting
            ));
        }
    } else {
        println!(
            "engagement  armed {} / engaged {} / awaiting {}                     OK",
            e.armed, e.engaged, e.awaiting
        );
    }

    // ---- stage 3: prose must not be masked ----------------------------------------
    let mut masked = 0usize;
    for probe in PROSE_PROBES {
        let before = backend.grammar_engagement();
        let out = backend
            .dispatch(
                &id,
                &InferenceInput::Text(render(&backend, &id, &tool_system(), probe)),
                &constrained_params(),
                &w,
            )
            .expect("a prose dispatch must complete");
        let pe = backend.grammar_engagement().since(before);
        let text = String::from_utf8_lossy(&out.bytes).to_string();
        // The control on the control: if nothing was armed for this turn, a clean prose
        // answer says nothing about over-eagerness.
        assert!(
            pe.armed > 0,
            "prose probe ran with NO grammar armed, so it cannot test masking: {probe:?}"
        );
        if pe.engaged > 0 {
            masked += 1;
            failures.push(format!(
                "prose-mask: prose was MASKED as a tool call. Probe {probe:?} engaged the \
                 grammar (engaged {}), and the answer came back as {text:?}. An over-eager \
                 trigger is a FAILURE of this gate, not a partial success.",
                pe.engaged
            ));
        }
    }
    if masked == 0 {
        println!(
            "prose-mask  {} probes, 0 masked                                    OK",
            PROSE_PROBES.len()
        );
    }

    // ---- stage 4: the emitted call is well-formed and grant-legal -------------------
    let parsed = kx_toolcall::parse_tool_call(
        constrained_text.as_bytes(),
        &w,
        kx_toolcall::max_args_bytes(&w),
    );
    match &parsed {
        Ok(Some(call)) => println!(
            "tool-call   {}@{} args {} bytes, grant-legal                       OK",
            call.name.0,
            call.version.0,
            call.args_bytes.len()
        ),
        Ok(None) => failures.push(format!(
            "tool-call: the model answered with PROSE on a turn that demanded a tool call. \
             Emitted: {constrained_text:?}"
        )),
        Err(e) => failures.push(format!(
            "tool-call: the emitted call was REFUSED by the parser ({e:?}). Emitted: \
             {constrained_text:?}"
        )),
    }

    // ---- the A/B, reported either way ----------------------------------------------
    // Not a pass/fail stage: a model may legitimately emit the same bytes both ways on a
    // task this simple. It is recorded because two byte-identical arms are their own
    // diagnosis, and reading it later without this line is impossible.
    let control = backend
        .dispatch(
            &id,
            &InferenceInput::Text(render(&backend, &id, &tool_system(), TOOL_TASK)),
            &control_params(),
            &w,
        )
        .expect("the control dispatch must complete");
    let control_text = String::from_utf8_lossy(&control.bytes).to_string();
    println!(
        "a/b         constrained {} control   (constrained={constrained_text:?} control={control_text:?})",
        if control_text == constrained_text {
            "==".to_string()
        } else {
            "!=".to_string()
        },
    );

    assert!(
        failures.is_empty(),
        "MODEL CONFORMANCE FAILED for {}:\n\n{}\n",
        model_path().display(),
        failures.join("\n\n")
    );
    println!("\nVERDICT: SUPPORTED");
}

/// Diagnostic: what does this model emit for the tool task with NO grammar armed?
///
/// The unconstrained emission is the ground truth a dialect and a grammar branch must be
/// built against. Reading it is the first step whenever the gate's engagement stage fails:
/// it separates "the model spoke a syntax we do not arm" from "the model did not try to
/// call a tool at all".
#[test]
#[ignore = "diagnostic; needs a real GGUF via KX_CONFORMANCE_GGUF"]
fn report_unconstrained_emission() {
    let (backend, id) = backend();
    let w = warrant(&id);
    for (label, prompt) in [("tool-task", TOOL_TASK), ("prose", PROSE_PROBES[0])] {
        let prompt = &render(&backend, &id, &tool_system(), prompt);
        let out = backend
            .dispatch(
                &id,
                &InferenceInput::Text(prompt.clone()),
                &control_params(),
                &w,
            )
            .expect("unconstrained dispatch");
        println!(
            "EMISSION[{label}] = {:?}",
            String::from_utf8_lossy(&out.bytes)
        );
    }
}
