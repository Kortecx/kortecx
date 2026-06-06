//! Unit + property tests for the planner: fail-closed decode (IMP-5), the
//! never-widen warrant property (D75), loop→shaper-not-cycle (D76), the
//! no-confidence-channel guard (D77), exact-role-equality (D70), and
//! deterministic lowering (D74).

use std::collections::{BTreeMap, BTreeSet};

use kx_content::ContentRef;
use kx_critic_types::{CheckSpec, SchemaSpec, SchemaTag};
use kx_mote::{
    EffectPattern, InferenceParams, LogicRef, ModelId, NdClass, PromptTemplateHash, RoleId,
    ToolName, ToolVersion,
};
use kx_warrant::{
    ExecutorClass, FsScope, InMemoryRoleRegistry, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    Role, ToolGrant, WarrantSpec,
};
use proptest::prelude::*;

use crate::{
    compile_plan, decode_loop_proposal, decode_plan, lower_loop_to_topology_decision, lower_plan,
    seed_from_plan_bytes, InMemoryRoleRecipes, LoopProposal, PlanError, PlanStep, PlanStepKind,
    RoleRecipe,
};

const SYSCALL: [u8; 32] = [7; 32];

/// A permissive parent warrant: grants tools `t1@1` and `t2@1`, no egress.
fn parent_warrant() -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    tool_grants.insert(ToolGrant {
        tool_id: ToolName("t1".into()),
        tool_version: ToolVersion("1".into()),
    });
    tool_grants.insert(ToolGrant {
        tool_id: ToolName("t2".into()),
        tool_version: ToolVersion("1".into()),
    });
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes(SYSCALL),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 4096,
            max_output_tokens: 1024,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 60_000,
            fd_count: 256,
            disk_bytes: 1 << 30,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A role whose spec narrows `parent` to the given tool grants (same syscall
/// profile — `intersect` requires equality — and no egress).
fn role_with_tools(name: &str, grants: &[(&str, &str)]) -> Role {
    let mut spec = parent_warrant();
    let mut tg = BTreeSet::new();
    for (id, ver) in grants {
        tg.insert(ToolGrant {
            tool_id: ToolName((*id).into()),
            tool_version: ToolVersion((*ver).into()),
        });
    }
    spec.tool_grants = tg;
    Role {
        name: name.into(),
        version: 1,
        spec,
        description: String::new(),
    }
}

fn recipe(
    nd_class: NdClass,
    capability: &str,
    tools: &[(&str, &str)],
    check: Option<CheckSpec>,
) -> RoleRecipe {
    let mut tool_contract = BTreeMap::new();
    for (id, ver) in tools {
        tool_contract.insert(ToolName((*id).into()), ToolVersion((*ver).into()));
    }
    RoleRecipe {
        logic_ref: LogicRef::from_bytes([0xA1; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0xB2; 32]),
        tool_contract,
        capability: ToolName(capability.into()),
        nd_class,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        inference_params: InferenceParams::default(),
        deterministic_check: check,
    }
}

/// Register a standard set of within-parent roles + recipes for the lowering
/// tests: `reader`/`summarizer`/`producer` (PURE, no tools) and `checker` (a
/// PURE deterministic critic carrying a Text-schema check).
fn standard_registries() -> (InMemoryRoleRegistry, InMemoryRoleRecipes) {
    let roles = InMemoryRoleRegistry::new();
    let recipes = InMemoryRoleRecipes::new();
    for name in ["reader", "summarizer", "producer"] {
        roles.register(RoleId(name.into()), role_with_tools(name, &[]));
        recipes.register(
            RoleId(name.into()),
            recipe(NdClass::Pure, "kx-model", &[], None),
        );
    }
    roles.register(RoleId("checker".into()), role_with_tools("checker", &[]));
    recipes.register(
        RoleId("checker".into()),
        recipe(
            NdClass::Pure,
            "kx-model",
            &[],
            Some(CheckSpec::Schema(SchemaSpec {
                expected: SchemaTag::Text,
            })),
        ),
    );
    (roles, recipes)
}

fn json(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

// ---------------------------------------------------------------------------
// decode (IMP-5)
// ---------------------------------------------------------------------------

#[test]
fn well_formed_plan_decodes() {
    let bytes = json(
        r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"read"},{"role":"summarizer","intent":"sum"}],"edges":[{"parent":0,"child":1}]}}"#,
    );
    let plan = decode_plan(&bytes, 8192).expect("a valid plan decodes");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.edges.len(), 1);
    assert_eq!(plan.steps[0].kind, PlanStepKind::Plain);
}

#[test]
fn think_preamble_then_plan_decodes() {
    // Qwen3 thinking-mode: a reasoning block precedes the plan JSON.
    let bytes = json(
        "<think>The user wants a read then a summary.</think>\n{\"plan\":{\"version\":1,\"steps\":[{\"role\":\"reader\",\"intent\":\"read\"},{\"role\":\"summarizer\",\"intent\":\"sum\"}],\"edges\":[{\"parent\":0,\"child\":1}]}}",
    );
    let plan = decode_plan(&bytes, 8192).expect("a plan after a think block decodes");
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.edges.len(), 1);
}

#[test]
fn unclosed_think_is_malformed() {
    // An unterminated reasoning block strips to "" ⇒ a plan is mandatory ⇒ Err.
    assert!(matches!(
        decode_plan(b"<think>reasoning with no closing tag", 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn oversize_with_think_still_oversize_on_original_bytes() {
    // The size cap is on the ORIGINAL bytes — a giant `<think>` block cannot
    // sneak past the budget by being stripped before the length check.
    let mut s = String::from("<think>");
    s.push_str(&"x".repeat(10_240));
    s.push_str("</think>{\"plan\":{\"version\":1,\"steps\":[{\"role\":\"r\",\"intent\":\"i\"}]}}");
    assert!(matches!(
        decode_plan(s.as_bytes(), 64),
        Err(PlanError::Oversize { .. })
    ));
}

#[test]
fn oversize_is_refused_before_parse() {
    // A 10 KiB buffer with a 4-byte cap must refuse on size alone (no parse).
    let big = vec![b'{'; 10_240];
    assert!(matches!(
        decode_plan(&big, 4),
        Err(PlanError::Oversize {
            got: 10_240,
            max: 4
        })
    ));
}

#[test]
fn malformed_inputs_fail_closed() {
    // truncated
    assert!(matches!(
        decode_plan(br#"{"plan":{"version":1,"steps":["#, 8192),
        Err(PlanError::Malformed { .. })
    ));
    // not an object
    assert!(matches!(
        decode_plan(b"[]", 8192),
        Err(PlanError::Malformed { .. })
    ));
    // missing the `plan` envelope key
    assert!(matches!(
        decode_plan(br#"{"steps":[]}"#, 8192),
        Err(PlanError::Malformed { .. })
    ));
    // trailing garbage after a valid envelope
    assert!(matches!(
        decode_plan(
            br#"{"plan":{"version":1,"steps":[{"role":"r","intent":"i"}]}} then prose"#,
            8192
        ),
        Err(PlanError::Malformed { .. })
    ));
    // non-UTF-8
    assert!(matches!(
        decode_plan(&[0xff, 0xfe, 0x00], 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn unknown_field_is_refused_no_confidence_channel_d77() {
    // deny_unknown_fields: a smuggled "confidence" score on a step is refused.
    // This is the structural proof that the plan schema offers NO score channel
    // a model could use to influence the deterministic promotion gate (D77).
    let bytes = json(
        r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"i","confidence":0.99}]}}"#,
    );
    assert!(matches!(
        decode_plan(&bytes, 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn unknown_version_zero_steps_and_caps_are_refused() {
    assert!(matches!(
        decode_plan(
            br#"{"plan":{"version":2,"steps":[{"role":"r","intent":"i"}]}}"#,
            8192
        ),
        Err(PlanError::UnknownVersion { version: 2 })
    ));
    assert!(matches!(
        decode_plan(br#"{"plan":{"version":1,"steps":[]}}"#, 8192),
        Err(PlanError::EmptyPlan)
    ));
    // > MAX_PLAN_STEPS steps.
    let mut steps = String::new();
    for _ in 0..=crate::MAX_PLAN_STEPS {
        steps.push_str(r#"{"role":"r","intent":"i"},"#);
    }
    steps.pop(); // trailing comma
    let bytes = json(&format!(r#"{{"plan":{{"version":1,"steps":[{steps}]}}}}"#));
    assert!(matches!(
        decode_plan(&bytes, 1 << 20),
        Err(PlanError::TooManySteps { .. })
    ));
}

proptest! {
    /// decode is total + panic-free over arbitrary bytes.
    #[test]
    fn decode_never_panics_on_arbitrary_bytes(bytes: Vec<u8>) {
        let _ = decode_plan(&bytes, 4096);
    }

    /// Deeply-nested input does not overflow the stack — serde_json's recursion
    /// limit makes it a bounded `Err`, never a panic.
    #[test]
    fn deep_nesting_is_bounded_err(depth in 1usize..50_000) {
        let mut s = String::from(r#"{"plan":{"version":1,"steps":"#);
        s.push_str(&"[".repeat(depth));
        let r = decode_plan(s.as_bytes(), 1 << 20);
        prop_assert!(r.is_err());
    }
}

// ---------------------------------------------------------------------------
// decode_loop_proposal (IMP-5) — the agentic-loop round trust boundary.
// Mirrors the decode_plan suite: same fail-closed discipline, distinct envelope.
// ---------------------------------------------------------------------------

#[test]
fn well_formed_loop_proposal_decodes() {
    let bytes = json(
        r#"{"loop_proposal":{"version":1,"next_steps":[{"role":"reader","intent":"again"},{"role":"summarizer","intent":"again"}]}}"#,
    );
    let proposal = decode_loop_proposal(&bytes, 8192).expect("a valid proposal decodes");
    assert_eq!(proposal.next_steps.len(), 2);
    assert_eq!(proposal.next_steps[0].role, "reader");
    assert_eq!(proposal.next_steps[0].kind, PlanStepKind::Plain);
}

#[test]
fn think_preamble_then_loop_proposal_decodes() {
    // Qwen3 thinking-mode: a reasoning block precedes the proposal JSON.
    let bytes = json(
        "<think>I should spawn a reader next.</think>\n{\"loop_proposal\":{\"version\":1,\"next_steps\":[{\"role\":\"reader\",\"intent\":\"again\"}]}}",
    );
    let proposal = decode_loop_proposal(&bytes, 8192).expect("a proposal after a think block");
    assert_eq!(proposal.next_steps.len(), 1);
}

#[test]
fn unclosed_think_loop_is_malformed() {
    // An unterminated reasoning block strips to "" ⇒ a proposal is mandatory ⇒ Err.
    assert!(matches!(
        decode_loop_proposal(b"<think>reasoning with no closing tag", 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn oversize_loop_with_think_still_oversize_on_original_bytes() {
    // The size cap is on the ORIGINAL bytes — a giant `<think>` block cannot
    // sneak past the budget by being stripped before the length check.
    let mut s = String::from("<think>");
    s.push_str(&"x".repeat(10_240));
    s.push_str("</think>{\"loop_proposal\":{\"version\":1,\"next_steps\":[{\"role\":\"r\",\"intent\":\"i\"}]}}");
    assert!(matches!(
        decode_loop_proposal(s.as_bytes(), 64),
        Err(PlanError::Oversize { .. })
    ));
}

#[test]
fn oversize_loop_is_refused_before_parse() {
    let big = vec![b'{'; 10_240];
    assert!(matches!(
        decode_loop_proposal(&big, 4),
        Err(PlanError::Oversize {
            got: 10_240,
            max: 4
        })
    ));
}

#[test]
fn malformed_loop_inputs_fail_closed() {
    // truncated
    assert!(matches!(
        decode_loop_proposal(br#"{"loop_proposal":{"version":1,"next_steps":["#, 8192),
        Err(PlanError::Malformed { .. })
    ));
    // not an object
    assert!(matches!(
        decode_loop_proposal(b"[]", 8192),
        Err(PlanError::Malformed { .. })
    ));
    // missing the `loop_proposal` envelope key (e.g. a bare plan smuggled in)
    assert!(matches!(
        decode_loop_proposal(br#"{"plan":{"version":1,"steps":[]}}"#, 8192),
        Err(PlanError::Malformed { .. })
    ));
    // trailing garbage after a valid envelope
    assert!(matches!(
        decode_loop_proposal(
            br#"{"loop_proposal":{"version":1,"next_steps":[{"role":"r","intent":"i"}]}} then prose"#,
            8192
        ),
        Err(PlanError::Malformed { .. })
    ));
    // non-UTF-8
    assert!(matches!(
        decode_loop_proposal(&[0xff, 0xfe, 0x00], 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn unknown_field_in_loop_step_refused_no_confidence_channel_d77() {
    // deny_unknown_fields on the reused PlanStep: a smuggled "confidence" on a
    // proposed step is refused — a loop round offers NO score channel either.
    let bytes = json(
        r#"{"loop_proposal":{"version":1,"next_steps":[{"role":"reader","intent":"i","confidence":0.99}]}}"#,
    );
    assert!(matches!(
        decode_loop_proposal(&bytes, 8192),
        Err(PlanError::Malformed { .. })
    ));
    // a smuggled sibling on the envelope itself is equally refused
    let bytes2 = json(
        r#"{"loop_proposal":{"version":1,"next_steps":[{"role":"r","intent":"i"}]},"score":1.0}"#,
    );
    assert!(matches!(
        decode_loop_proposal(&bytes2, 8192),
        Err(PlanError::Malformed { .. })
    ));
}

#[test]
fn unknown_version_empty_and_cap_refused_loop() {
    assert!(matches!(
        decode_loop_proposal(
            br#"{"loop_proposal":{"version":2,"next_steps":[{"role":"r","intent":"i"}]}}"#,
            8192
        ),
        Err(PlanError::UnknownVersion { version: 2 })
    ));
    assert!(matches!(
        decode_loop_proposal(br#"{"loop_proposal":{"version":1,"next_steps":[]}}"#, 8192),
        Err(PlanError::EmptyPlan)
    ));
    // > MAX_LOOP_STEPS steps.
    let mut steps = String::new();
    for _ in 0..=crate::MAX_LOOP_STEPS {
        steps.push_str(r#"{"role":"r","intent":"i"},"#);
    }
    steps.pop(); // trailing comma
    let bytes = json(&format!(
        r#"{{"loop_proposal":{{"version":1,"next_steps":[{steps}]}}}}"#
    ));
    assert!(matches!(
        decode_loop_proposal(&bytes, 1 << 20),
        Err(PlanError::TooManySteps { .. })
    ));
}

#[test]
fn loop_proposal_decodes_then_lowers_to_topology_decision() {
    // End-to-end of the pure path: model bytes → decode → lower → a content-
    // addressed TopologyDecision (the exact fact a ROND shaper commits).
    let (_roles, recipes) = standard_registries();
    let bytes = json(
        r#"{"loop_proposal":{"version":1,"next_steps":[{"role":"reader","intent":"again"},{"role":"summarizer","intent":"again"}]}}"#,
    );
    let proposal = decode_loop_proposal(&bytes, 8192).expect("decodes");
    let td = lower_loop_to_topology_decision(&proposal, &recipes).expect("lowers");
    assert_eq!(td.children.len(), 2);
    assert_eq!(&td.children[0].role_id.0, "reader");
    assert_eq!(td.hash(), td.hash()); // deterministic content address
}

proptest! {
    /// decode_loop_proposal is total + panic-free over arbitrary bytes.
    #[test]
    fn decode_loop_proposal_never_panics_on_arbitrary_bytes(bytes: Vec<u8>) {
        let _ = decode_loop_proposal(&bytes, 4096);
    }

    /// Deeply-nested input is a bounded `Err`, never a stack overflow.
    #[test]
    fn loop_deep_nesting_is_bounded_err(depth in 1usize..50_000) {
        let mut s = String::from(r#"{"loop_proposal":{"version":1,"next_steps":"#);
        s.push_str(&"[".repeat(depth));
        let r = decode_loop_proposal(s.as_bytes(), 1 << 20);
        prop_assert!(r.is_err());
    }
}

// ---------------------------------------------------------------------------
// lowering: role selection + warrant narrowing (D75) + exact equality (D70)
// ---------------------------------------------------------------------------

#[test]
fn compiles_a_chain_deterministically() {
    let (roles, recipes) = standard_registries();
    let bytes = json(
        r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"read"},{"role":"summarizer","intent":"sum"}],"edges":[{"parent":0,"child":1}]}}"#,
    );
    let plan = decode_plan(&bytes, 8192).unwrap();
    let seed = seed_from_plan_bytes(&bytes);
    let parent = parent_warrant();

    let a = compile_plan(&plan, seed, &parent, &roles, &recipes).expect("compiles");
    let b = compile_plan(&plan, seed, &parent, &roles, &recipes).expect("compiles");
    assert_eq!(a.motes.len(), 2);
    // Lowering + compile are pure: same plan + seed ⇒ identical MoteIds.
    let ids_a: Vec<_> = a.motes.iter().map(|m| m.mote.id).collect();
    let ids_b: Vec<_> = b.motes.iter().map(|m| m.mote.id).collect();
    assert_eq!(ids_a, ids_b);
    // Each produced step's warrant is no wider than the parent (D75).
    for m in &a.motes {
        assert!(m.warrant.tool_grants.is_subset(&parent.tool_grants));
    }
}

#[test]
fn deterministic_critic_chain_auto_wires_producer_edge() {
    let (roles, recipes) = standard_registries();
    // producer (step 0) → checker (step 1, deterministic_critic of 0). No edge
    // declared: lowering auto-wires producer→critic so compile's precedence
    // check passes.
    let bytes = json(
        r#"{"plan":{"version":1,"steps":[{"role":"producer","intent":"make"},{"role":"checker","intent":"check","kind":"deterministic_critic","producer":0}]}}"#,
    );
    let plan = decode_plan(&bytes, 8192).unwrap();
    let cw = compile_plan(&plan, 1, &parent_warrant(), &roles, &recipes).expect("compiles");
    assert_eq!(cw.motes.len(), 2);
}

#[test]
fn unknown_role_and_recipe_and_near_miss_refuse_d70() {
    let (roles, recipes) = standard_registries();
    let parent = parent_warrant();
    // Exact-equality only: a near-miss name does NOT fuzzy-match a real role.
    for bad in ["Reader", "reader ", "summarise", "READER"] {
        let bytes = json(&format!(
            r#"{{"plan":{{"version":1,"steps":[{{"role":"{bad}","intent":"i"}}]}}}}"#
        ));
        let plan = decode_plan(&bytes, 8192).unwrap();
        assert!(matches!(
            lower_plan(&plan, 1, &parent, &roles, &recipes),
            Err(PlanError::UnknownRole(_))
        ));
    }
    // A role registered for the warrant but with no recipe → UnknownRecipe.
    let roles2 = InMemoryRoleRegistry::new();
    roles2.register(RoleId("orphan".into()), role_with_tools("orphan", &[]));
    let recipes2 = InMemoryRoleRecipes::new();
    let bytes = json(r#"{"plan":{"version":1,"steps":[{"role":"orphan","intent":"i"}]}}"#);
    let plan = decode_plan(&bytes, 8192).unwrap();
    assert!(matches!(
        lower_plan(&plan, 1, &parent, &roles2, &recipes2),
        Err(PlanError::UnknownRecipe(_))
    ));
}

#[test]
fn ungrantable_tool_in_recipe_is_refused() {
    // A role whose warrant grants only t1, but whose recipe declares t2 → the
    // step could never legally call t2 → refused up front (IMP-5).
    let roles = InMemoryRoleRegistry::new();
    roles.register(RoleId("r".into()), role_with_tools("r", &[("t1", "1")]));
    let recipes = InMemoryRoleRecipes::new();
    recipes.register(
        RoleId("r".into()),
        recipe(NdClass::Pure, "t2", &[("t2", "1")], None),
    );
    let bytes = json(r#"{"plan":{"version":1,"steps":[{"role":"r","intent":"i"}]}}"#);
    let plan = decode_plan(&bytes, 8192).unwrap();
    assert!(matches!(
        lower_plan(&plan, 1, &parent_warrant(), &roles, &recipes),
        Err(PlanError::UngrantableTool { .. })
    ));
}

#[test]
fn invalid_producer_is_refused() {
    let (roles, recipes) = standard_registries();
    // critic with no producer
    let b1 =
        json(r#"{"plan":{"version":1,"steps":[{"role":"checker","intent":"c","kind":"critic"}]}}"#);
    assert!(matches!(
        lower_plan(
            &decode_plan(&b1, 8192).unwrap(),
            1,
            &parent_warrant(),
            &roles,
            &recipes
        ),
        Err(PlanError::InvalidProducer { .. })
    ));
    // critic whose producer does not precede it (producer == self)
    let b2 = json(
        r#"{"plan":{"version":1,"steps":[{"role":"checker","intent":"c","kind":"critic","producer":0}]}}"#,
    );
    assert!(matches!(
        lower_plan(
            &decode_plan(&b2, 8192).unwrap(),
            1,
            &parent_warrant(),
            &roles,
            &recipes
        ),
        Err(PlanError::InvalidProducer { .. })
    ));
}

// ---------------------------------------------------------------------------
// loop → shaper, never a cycle (D76)
// ---------------------------------------------------------------------------

#[test]
fn loop_lowers_to_topology_decision_not_a_cycle() {
    let (_roles, recipes) = standard_registries();
    let proposal = LoopProposal {
        next_steps: vec![
            PlanStep {
                role: "reader".into(),
                intent: "again".into(),
                kind: PlanStepKind::Plain,
                producer: None,
            },
            PlanStep {
                role: "summarizer".into(),
                intent: "again".into(),
                kind: PlanStepKind::Plain,
                producer: None,
            },
        ],
    };
    let td = lower_loop_to_topology_decision(&proposal, &recipes).expect("lowers");
    assert_eq!(td.children.len(), 2);
    assert_eq!(&td.children[0].role_id.0, "reader");
    // Deterministic content address (the shaper commits this).
    assert_eq!(td.hash(), td.hash());
}

#[test]
fn a_plan_authored_as_a_cycle_is_refused_by_compile() {
    let (roles, recipes) = standard_registries();
    // An agentic loop expressed as a DAG back-edge (0→1, 1→0) must be refused —
    // loops belong to a shaper (above), never a cycle.
    let bytes = json(
        r#"{"plan":{"version":1,"steps":[{"role":"reader","intent":"a"},{"role":"summarizer","intent":"b"}],"edges":[{"parent":0,"child":1},{"parent":1,"child":0}]}}"#,
    );
    let plan = decode_plan(&bytes, 8192).unwrap();
    assert!(matches!(
        compile_plan(&plan, 1, &parent_warrant(), &roles, &recipes),
        Err(PlanError::Compile(kx_workflow::CompileError::Cycle(_)))
    ));
}

// ---------------------------------------------------------------------------
// never-widen warrant property (D75) — proptest over arbitrary role grants
// ---------------------------------------------------------------------------

proptest! {
    /// For ANY role tool-grant subset of a fixed universe, lowering either
    /// succeeds with a warrant whose grants ⊆ parent's, or (when the role
    /// proposes an ungranted tool) refuses with `Ungrantable` — NEVER a step
    /// with authority wider than the parent. The recipe declares no tools (so
    /// the UngrantableTool check is inert and we isolate the warrant intersect).
    #[test]
    fn role_selection_never_widens_parent_warrant(
        grants in proptest::collection::vec(
            proptest::sample::select(vec!["t1", "t2", "t3", "t4"]),
            0..5,
        )
    ) {
        let parent = parent_warrant(); // grants t1,t2
        let grant_pairs: Vec<(&str, &str)> = grants.iter().map(|t| (*t, "1")).collect();
        let roles = InMemoryRoleRegistry::new();
        roles.register(RoleId("x".into()), role_with_tools("x", &grant_pairs));
        let recipes = InMemoryRoleRecipes::new();
        recipes.register(RoleId("x".into()), recipe(NdClass::Pure, "kx-model", &[], None));

        let bytes = json(r#"{"plan":{"version":1,"steps":[{"role":"x","intent":"i"}]}}"#);
        let plan = decode_plan(&bytes, 8192).unwrap();
        let within = grant_pairs.iter().all(|(id, _)| *id == "t1" || *id == "t2");

        match compile_plan(&plan, 1, &parent, &roles, &recipes) {
            Ok(cw) => {
                prop_assert!(within, "a wider role must never compile to a warrant");
                for m in &cw.motes {
                    prop_assert!(m.warrant.tool_grants.is_subset(&parent.tool_grants));
                }
            }
            Err(PlanError::Ungrantable { .. }) => {
                prop_assert!(!within, "a within-parent role must not be refused");
            }
            Err(other) => prop_assert!(false, "unexpected error: {other:?}"),
        }
    }
}
