//! Model-free invariants — run in `cargo test --workspace` (no GGUF needed).
//!
//! - **E (identity)** — reuse the recipe, never the result: identical `MoteDef`s
//!   ⇒ identical `MoteId` (recipe reuse); a changed param or prompt ⇒ a different
//!   `MoteId` (a fresh call). Identity-bearing fields are integers (no float).
//! - **F** — step capture: `Full` retains reasoning/thinking; `ActionsOnly`
//!   strips them at the boundary, keeping only the action join key.
//! - **H** — graph-RAG: the committed retrieval fact is the ordered refs only;
//!   the same neighbours with different float scores ⇒ identical fact (SN-8).
//! - **I** — no float on identity: the integer `temperature_bps` participates in
//!   `MoteId`; similarity scores never do (H).

#![allow(clippy::unwrap_used)]

use kx_capture::{CaptureConsent, InMemoryCaptureStore, StepRecord};
use kx_content::ContentRef;
use kx_dataset::Hit;
use kx_mote::{ModelId, NdClass};
use kx_model_harness::{harness_warrant, workflows};
use kx_workflow::{encode_retrieval_fact, retrieval_result_ref};

fn model_id() -> ModelId {
    ModelId("test-model:q4_k_m:deadbeefcafef00d".into())
}

// ---- E (identity) + I (integer temperature participates) -------------------

#[test]
fn identical_recipe_yields_identical_mote_id() {
    let m = model_id();
    let w = harness_warrant(&m, 64, 60_000);
    let a = workflows::model_chain(&m, &w, "same prompt", workflows::greedy(32), NdClass::Pure);
    let b = workflows::model_chain(&m, &w, "same prompt", workflows::greedy(32), NdClass::Pure);
    assert_eq!(
        a.motes[0].mote.id, b.motes[0].mote.id,
        "identical MoteDef ⇒ identical MoteId (recipe reuse)"
    );
}

#[test]
fn changed_temperature_yields_new_mote_id() {
    let m = model_id();
    let w = harness_warrant(&m, 64, 60_000);
    let greedy = workflows::model_chain(&m, &w, "p", workflows::greedy(32), NdClass::Pure);
    // Same everything except the integer temperature_bps (greedy 0 vs sampled).
    let sampled =
        workflows::model_chain(&m, &w, "p", workflows::sampled(32, 0), NdClass::ReadOnlyNondet);
    assert_ne!(
        greedy.motes[0].mote.id, sampled.motes[0].mote.id,
        "a different (integer) temperature_bps ⇒ a different MoteId (D50; no float on identity)"
    );
}

#[test]
fn changed_prompt_yields_new_mote_id() {
    let m = model_id();
    let w = harness_warrant(&m, 64, 60_000);
    let a = workflows::model_chain(&m, &w, "prompt A", workflows::greedy(32), NdClass::Pure);
    let b = workflows::model_chain(&m, &w, "prompt B", workflows::greedy(32), NdClass::Pure);
    assert_ne!(
        a.motes[0].mote.id, b.motes[0].mote.id,
        "the prompt is identity-bearing (carried in config_subset) ⇒ different prompt, fresh call"
    );
}

#[test]
fn different_model_quant_yields_new_mote_id() {
    let w1 = harness_warrant(&ModelId("qwen:q4_k_m:aaaa".into()), 64, 60_000);
    let w2 = harness_warrant(&ModelId("qwen:q8_0:aaaa".into()), 64, 60_000);
    let a = workflows::model_chain(
        &ModelId("qwen:q4_k_m:aaaa".into()),
        &w1,
        "p",
        workflows::greedy(32),
        NdClass::Pure,
    );
    let b = workflows::model_chain(
        &ModelId("qwen:q8_0:aaaa".into()),
        &w2,
        "p",
        workflows::greedy(32),
        NdClass::Pure,
    );
    assert_ne!(
        a.motes[0].mote.id, b.motes[0].mote.id,
        "a different model/quant ⇒ a different ModelId ⇒ a different MoteId (D50)"
    );
}

// ---- F (step capture: Full vs ActionsOnly) ---------------------------------

#[test]
fn capture_full_retains_reasoning_actions_only_strips() {
    let mote_id = kx_mote::MoteId::from_bytes([7; 32]);
    let input = ContentRef::of(b"the prompt");
    let output = ContentRef::of(b"the action/result");
    let reasoning = ContentRef::of(b"the model's reasoning trace");
    let thinking = ContentRef::of(b"the model's <think> block");
    let full = StepRecord::full(
        mote_id,
        Some(input),
        Some(output),
        Some(reasoning),
        Some(thinking),
    );

    // Full consent retains everything.
    let mut full_store = InMemoryCaptureStore::new(CaptureConsent::full());
    full_store.record(full.clone());
    let got = full_store.get(&mote_id).unwrap();
    assert_eq!(got.reasoning_ref, Some(reasoning), "Full retains reasoning");
    assert_eq!(got.thinking_ref, Some(thinking), "Full retains thinking");
    assert_eq!(got.output_ref, Some(output));

    // ActionsOnly strips the opt-in fields at the boundary, keeps the action key.
    let mut actions_store = InMemoryCaptureStore::new(CaptureConsent::actions_only());
    actions_store.record(full);
    let got = actions_store.get(&mote_id).unwrap();
    assert_eq!(got.reasoning_ref, None, "ActionsOnly strips reasoning");
    assert_eq!(got.thinking_ref, None, "ActionsOnly strips thinking");
    assert_eq!(got.input_ref, None, "ActionsOnly strips input");
    assert_eq!(
        got.output_ref,
        Some(output),
        "ActionsOnly keeps the committed action join key"
    );
}

// ---- H (graph-RAG: ordered refs, scores excluded) --------------------------

#[test]
fn retrieval_fact_excludes_scores() {
    let ids = [ContentRef::of(b"doc-1"), ContentRef::of(b"doc-2"), ContentRef::of(b"doc-3")];
    let high = ids.iter().map(|&id| Hit { id, score: 0.99 }).collect::<Vec<_>>();
    let low = ids.iter().map(|&id| Hit { id, score: 0.01 }).collect::<Vec<_>>();

    assert_eq!(
        encode_retrieval_fact(&high),
        encode_retrieval_fact(&low),
        "same neighbour ids, different float scores ⇒ identical committed fact"
    );
    assert_eq!(
        retrieval_result_ref(&high),
        retrieval_result_ref(&low),
        "the retrieval result_ref is over ids only — scores never reach identity (SN-8)"
    );

    // A different neighbour set DOES change the fact (the ids are what matter).
    let other = [ContentRef::of(b"doc-9")];
    let other_hits = other.iter().map(|&id| Hit { id, score: 0.99 }).collect::<Vec<_>>();
    assert_ne!(retrieval_result_ref(&high), retrieval_result_ref(&other_hits));
}
