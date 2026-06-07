//! PR-3 (AL2) — "the model re-plans on failure" with a REAL GGUF model (run with
//! `--features with-model`, host + Metal). The live counterpart of
//! `replan_loop_e2e.rs`. A real model's steps commit normally, so this exercises
//! the re-plan DRIVER on the no-correction path (`rounds_used == 1`, equivalent to
//! the PR-2 single round) end-to-end through `drive_replan_loop`; the
//! failure → correction → budget → escalate paths are covered DETERMINISTICALLY in
//! `replan_loop_e2e.rs` (a real-model child failure cannot be injected reproducibly).
//!
//! Hard invariants (hold for ANY model output):
//! - the loop **completes** (never a panic / abort / hang) and `rounds_used >= 1`;
//! - the round-0 shaper reaches a **terminal** state (Committed or Failed);
//! - **R49** — two cold re-folds of the committed journal reproduce byte-identical
//!   `MoteId`s (the model's decision is a replayed fact, never re-sampled).

#![cfg(feature = "with-model")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use kx_journal::SqliteJournal;
use kx_model_harness::{harness_warrant, model_id_for, workflows, Harness, LoopBudget};
use kx_mote::{
    EffectPattern, LogicRef, ModelId, MoteDef, NdClass, PromptTemplateHash, RoleId, ToolName,
};
use kx_planner::{InMemoryRoleRecipes, RoleRecipe, RoleRecipeResolver};
use kx_projection::{
    DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver, MoteState,
    Projection,
};
use kx_runtime::config::Mode;
use kx_runtime::RuntimeConfig;
use kx_warrant::{InMemoryRoleRegistry, Role, WarrantSpec};

const ROLES: [&str; 2] = ["reader", "writer"];
const SHAPER_SEED: u8 = 0x77;

const PLAN_PROMPT: &str = "You are a planning agent. Output ONLY one JSON object, \
no prose, of EXACTLY this shape: \
{\"loop_proposal\":{\"version\":1,\"next_steps\":[{\"role\":\"reader\",\"intent\":\"read the input\"}]}}. \
The only allowed role values are \"reader\" and \"writer\". Do not add any other fields.";

fn gguf() -> std::path::PathBuf {
    kx_model_harness::default_gguf_path()
}

fn config(dir: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("j.sqlite"),
        content_root: dir.join("c"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every: None,
        audit_log: None,
    }
}

fn recipes(model_id: &ModelId) -> Arc<dyn RoleRecipeResolver> {
    let r = InMemoryRoleRecipes::new();
    for (i, name) in ROLES.iter().enumerate() {
        let tag = u8::try_from(i).unwrap();
        r.register(
            RoleId((*name).into()),
            RoleRecipe {
                logic_ref: LogicRef::from_bytes([0x90 + tag; 32]),
                model_id: model_id.clone(),
                prompt_template_hash: PromptTemplateHash::from_bytes([0xA0 + tag; 32]),
                tool_contract: BTreeMap::new(),
                capability: ToolName("kx-model".into()),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
                inference_params: kx_mote::InferenceParams::default(),
                deterministic_check: None,
            },
        );
    }
    Arc::new(r)
}

/// Cold re-fold the committed journal through a fresh materializer — the shipped
/// recovery path; decodes the committed decision fact, never calls a model.
fn cold_fold(cfg: &RuntimeConfig, shaper_def: &MoteDef, warrant: &WarrantSpec) -> Projection {
    let store = Arc::new(kx_content::LocalFsContentStore::open(&cfg.content_root).unwrap());
    let journal = SqliteJournal::open(&cfg.journal_path).unwrap();
    let def_registry = InMemoryMoteDefRegistry::new();
    def_registry.register(shaper_def.clone());
    let role_registry = InMemoryRoleRegistry::new();
    for r in ROLES {
        role_registry.register(
            RoleId(r.into()),
            Role {
                name: r.into(),
                version: 1,
                spec: warrant.clone(),
                description: String::new(),
            },
        );
    }
    let materializer = Box::new(DefaultTopologyMaterializer::new(
        store,
        Arc::new(def_registry),
        Arc::new(role_registry),
        InheritFromShaperResolver,
    ));
    Projection::from_journal_with_materializer(&journal, materializer).unwrap()
}

#[test]
fn a_real_model_drives_the_replan_loop() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    let warrant = harness_warrant(&model_id, 256, 120_000);
    let wf = workflows::loop_shaper(&model_id, &warrant, PLAN_PROMPT, SHAPER_SEED);
    let shaper_def = wf.motes[0].mote.def.clone();
    let shaper_id = wf.shaper_id;

    let harness = Harness::open(&cfg, &gguf(), model_id).unwrap();
    let outcome = harness
        .drive_replan_loop(&cfg, &wf, recipes(&harness.model_id), LoopBudget::default())
        .expect("the re-plan loop completes (a refused proposal dead-letters, never aborts)");

    assert!(outcome.rounds_used >= 1, "at least the initial round ran");

    // The round-0 shaper is terminal (committed a decision, or dead-lettered).
    let p1 = cold_fold(&cfg, &shaper_def, &warrant);
    let shaper_state = p1.state_of(&shaper_id);
    assert!(
        matches!(shaper_state, MoteState::Committed | MoteState::Failed),
        "round-0 shaper terminal: {shaper_state:?}"
    );

    // R49: a second cold re-fold reproduces byte-identical identities.
    let p2 = cold_fold(&cfg, &shaper_def, &warrant);
    let ids1: Vec<_> = p1.iter_motes().map(|(id, _)| id).collect();
    let ids2: Vec<_> = p2.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(ids1, ids2, "R49: cold re-folds are identical");

    eprintln!(
        "replan loop: rounds_used={}, shaper={shaper_state:?}, escalation={:?}, committed={}/{}",
        outcome.rounds_used, outcome.escalation, outcome.run.committed, outcome.run.total
    );
}
