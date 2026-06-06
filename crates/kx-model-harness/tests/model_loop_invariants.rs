//! PR-2 (F-4) — "the model drives the loop" with a REAL GGUF model (run with
//! `--features with-model`, host + Metal). The live counterpart of
//! `model_loop_e2e.rs`: a real model produces the `loop_proposal` envelope, which
//! is decoded + lowered + committed as the shaper's `TopologyDecision` fact and
//! materialized into children that execute.
//!
//! Hard invariants (hold for ANY model output — a cooperative model spawns
//! children; a model that emits a malformed / un-grantable proposal dead-letters
//! the shaper fail-closed, PR-1):
//! - the run **completes** (never a panic / abort / hang);
//! - the shaper reaches a **terminal** state (Committed with a decision, or Failed
//!   dead-letter);
//! - **R49** — two cold re-folds of the committed journal reproduce byte-identical
//!   `MoteId`s (the model's decision is a captured fact, replayed, never
//!   re-sampled; the materializer never calls the model).

#![cfg(feature = "with-model")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use kx_journal::SqliteJournal;
use kx_model_harness::evidence::Evidence;
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

/// Roles the planner is instructed to choose from (the recipe allowlist).
const ROLES: [&str; 2] = ["reader", "writer"];
const SHAPER_SEED: u8 = 0x73;

const PLAN_PROMPT: &str = "You are a planning agent. Output ONLY one JSON object, \
no prose, of EXACTLY this shape: \
{\"loop_proposal\":{\"version\":1,\"next_steps\":[{\"role\":\"reader\",\"intent\":\"read the input\"},{\"role\":\"writer\",\"intent\":\"write a summary\"}]}}. \
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

/// Cold re-fold the committed journal through a fresh materializer (resolving the
/// allowlisted roles) — the shipped recovery path; decodes the committed decision
/// fact, never calls a model.
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
fn a_real_model_drives_the_topology_loop() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).unwrap();
    // Generous output budget (the JSON envelope) + wall-clock for CPU/Metal decode.
    let warrant = harness_warrant(&model_id, 256, 120_000);
    let wf = workflows::loop_shaper(&model_id, &warrant, PLAN_PROMPT, SHAPER_SEED);
    let shaper_def = wf.motes[0].mote.def.clone();
    let shaper_id = wf.shaper_id;

    let harness = Harness::open(&cfg, &gguf(), model_id).unwrap();
    let outcome = harness
        .drive_model_loop(&cfg, &wf, recipes(&harness.model_id), LoopBudget::default())
        .expect("the model-driven loop completes (a refused proposal dead-letters, never aborts)");

    // The shaper is terminal: it either committed a decision (children spawned) or
    // was dead-lettered fail-closed. Never a hang.
    let p1 = cold_fold(&cfg, &shaper_def, &warrant);
    let shaper_state = p1.state_of(&shaper_id);
    assert!(
        matches!(shaper_state, MoteState::Committed | MoteState::Failed),
        "the shaper is terminal after the model drove the decision: {shaper_state:?}"
    );

    // R49: a second cold re-fold reproduces byte-identical identities (the model's
    // choice is a replayed fact, never re-sampled).
    let p2 = cold_fold(&cfg, &shaper_def, &warrant);
    let ids1: Vec<_> = p1.iter_motes().map(|(id, _)| id).collect();
    let ids2: Vec<_> = p2.iter_motes().map(|(id, _)| id).collect();
    assert_eq!(ids1, ids2, "R49: cold re-folds are identical");

    // When the model cooperated, the children materialized under the shaper.
    let children: Vec<_> = p1
        .iter_motes()
        .map(|(id, _)| id)
        .filter(|id| *id != shaper_id)
        .collect();
    if shaper_state == MoteState::Committed {
        assert!(!children.is_empty(), "a committed decision spawns ≥1 child");
        for c in &children {
            assert_eq!(
                p1.parents_of(c)[0].0,
                shaper_id,
                "child parent = the shaper"
            );
        }
    }

    eprintln!(
        "model-driven loop: shaper={shaper_state:?}, children={}, committed={}/{}",
        children.len(),
        outcome.committed,
        outcome.total
    );

    if let Some(stamp) = std::env::var("KX_RUNSTAMP").ok() {
        if let Ok(ev) = Evidence::open(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
            &stamp,
        ) {
            let _ = ev.write_str(
                "F4_model_loop",
                "outcome.txt",
                &format!(
                    "shaper={shaper_state:?}\nchildren={}\ncommitted={}/{}\n",
                    children.len(),
                    outcome.committed,
                    outcome.total
                ),
            );
        }
    }
}
