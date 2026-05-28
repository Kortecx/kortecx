//! The canonical demo workflow — a small but non-trivial Mote DAG exercising
//! every nd_class and commit pattern the P1 runtime must handle:
//!
//! ```text
//! M1   PURE root (deterministic compute)
//!  ├─> S    READ-ONLY-NONDET topology shaper ──> W0, W1 (PURE workers, materialized)
//!  ├─> M2   READ-ONLY-NONDET (model sample; reads M1)
//!  ├─> Wstc WORLD-MUTATING StageThenCommit (reads M1)     [scenario-1 crash]
//!  └─> M3   WORLD-MUTATING ValidateThenCommit (reads M2)  [scenario-2 crash]
//!        └─> M3c PURE critic (critic_for = M3; terminates the trust chain, R-9)
//! ```
//!
//! All identities are derived from fixed bytes so the workflow — and therefore
//! the journal it produces — is byte-identical across runs, processes, and
//! machines (the precondition for the kill-and-replay exit-gate assertions).

use std::collections::BTreeMap;

use kx_content::ContentRef;
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

/// One Mote of the demo workflow, paired with the warrant it runs under and
/// the capability its effect dispatches through (WORLD-MUTATING / READ-ONLY-
/// NONDET only; PURE Motes ignore the capability).
#[derive(Debug, Clone)]
pub struct WorkflowMote {
    /// The Mote to run.
    pub mote: Mote,
    /// The resolved warrant for this Mote.
    pub warrant: WarrantSpec,
    /// The capability a WM/ROND dispatch routes through.
    pub capability: ToolName,
}

/// The fully-built demo workflow.
#[derive(Debug, Clone)]
pub struct DemoWorkflow {
    /// Motes in submission order.
    pub motes: Vec<WorkflowMote>,
    /// The `StageThenCommit` Mote the scenario-1 crash targets.
    pub stc_crash_target: MoteId,
    /// The `ValidateThenCommit` Mote the scenario-2 crash targets.
    pub vtc_crash_target: MoteId,
    /// The READ-ONLY-NONDET topology shaper. Its committed `TopologyDecision`
    /// spawns worker children that must re-materialize identically on replay.
    pub shaper_id: MoteId,
}

impl DemoWorkflow {
    /// Build the canonical demo workflow.
    #[must_use]
    pub fn canonical() -> Self {
        let cap = ToolName("demo-tool".into());

        // M1 — PURE root.
        let m1 = pure_mote(0x01, &[]);
        // S — READ-ONLY-NONDET topology shaper, reads M1; commits a
        // TopologyDecision that spawns PURE worker children.
        let shaper = shaper_mote(0x06, &[data_parent(&m1)]);
        // M2 — READ-ONLY-NONDET, reads M1.
        let m2 = nondet_mote(
            0x02,
            NdClass::ReadOnlyNondet,
            EffectPattern::IdempotentByConstruction,
            None,
            &[data_parent(&m1)],
        );
        // Wstc — WORLD-MUTATING StageThenCommit, reads M1 (scenario-1 crash).
        let wstc = nondet_mote(
            0x03,
            NdClass::WorldMutating,
            EffectPattern::StageThenCommit,
            None,
            &[data_parent(&m1)],
        );
        // M3 — WORLD-MUTATING ValidateThenCommit, reads M2 (scenario-2 crash).
        let m3 = nondet_mote(
            0x04,
            NdClass::WorldMutating,
            EffectPattern::ValidateThenCommit,
            None,
            &[data_parent(&m2)],
        );
        // M3c — PURE critic for M3 (terminates the trust chain, R-9).
        let m3c = pure_mote_with_critic(0x05, Some(m3.id), &[data_parent(&m3)]);

        let stc_crash_target = wstc.id;
        let vtc_crash_target = m3.id;
        let shaper_id = shaper.id;

        let permissive = permissive_warrant();
        let motes = vec![
            WorkflowMote {
                mote: m1,
                warrant: permissive.clone(),
                capability: cap.clone(),
            },
            WorkflowMote {
                mote: shaper,
                warrant: permissive.clone(),
                capability: cap.clone(),
            },
            WorkflowMote {
                mote: m2,
                warrant: permissive.clone(),
                capability: cap.clone(),
            },
            WorkflowMote {
                mote: wstc,
                warrant: permissive.clone(),
                capability: cap.clone(),
            },
            WorkflowMote {
                mote: m3,
                warrant: permissive.clone(),
                capability: cap.clone(),
            },
            WorkflowMote {
                mote: m3c,
                warrant: permissive,
                capability: cap,
            },
        ];

        Self {
            motes,
            stc_crash_target,
            vtc_crash_target,
            shaper_id,
        }
    }

    /// The `mote_id → Mote` map `run_wm_mote` / `redispatch_wm_mote` consult to
    /// find a producer's sibling critic.
    #[must_use]
    pub fn submission_motes(&self) -> BTreeMap<MoteId, Mote> {
        self.motes
            .iter()
            .map(|w| (w.mote.id, w.mote.clone()))
            .collect()
    }
}

/// A permissive warrant — the demo runs in a single trusted process, so every
/// axis is wide open. (The broker/executor seams still enforce structurally;
/// the warrant just doesn't narrow anything for the demo.)
fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: std::collections::BTreeSet::new(),
        // Positive model-route limits: the topology materializer narrows the
        // shaper's warrant against each child role via `kx_warrant::intersect`,
        // which rejects a zeroed model route as invalid.
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 4096,
            max_output_tokens: 4096,
            max_calls: 16,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

/// A `MoteDef` template parameterized by the bytes that matter for the demo.
fn mote_def(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
) -> MoteDef {
    MoteDef {
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        nd_class,
        config_subset: BTreeMap::new(),
        effect_pattern,
        critic_for,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

fn build_mote(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    parents: &[ParentRef],
) -> Mote {
    let def = mote_def(seed, nd_class, effect_pattern, critic_for);
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents
            .iter()
            .copied()
            .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

fn pure_mote(seed: u8, parents: &[ParentRef]) -> Mote {
    build_mote(
        seed,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        None,
        parents,
    )
}

fn pure_mote_with_critic(seed: u8, critic_for: Option<MoteId>, parents: &[ParentRef]) -> Mote {
    build_mote(
        seed,
        NdClass::Pure,
        EffectPattern::IdempotentByConstruction,
        critic_for,
        parents,
    )
}

fn nondet_mote(
    seed: u8,
    nd_class: NdClass,
    effect_pattern: EffectPattern,
    critic_for: Option<MoteId>,
    parents: &[ParentRef],
) -> Mote {
    build_mote(seed, nd_class, effect_pattern, critic_for, parents)
}

/// A READ-ONLY-NONDET topology shaper (`is_topology_shaper = true`). R-14
/// forbids WORLD-MUTATING shapers; ROND is permitted (topology emission is a
/// nondet read of workflow state, not an external mutation).
fn shaper_mote(seed: u8, parents: &[ParentRef]) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([seed; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([seed; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::ReadOnlyNondet,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: true,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([seed; 32]),
        GraphPosition(vec![seed]),
        parents
            .iter()
            .copied()
            .collect::<SmallVec<[ParentRef; 4]>>(),
    )
}

/// A data-dependency parent edge on `parent`.
fn data_parent(parent: &Mote) -> ParentRef {
    ParentRef {
        parent_id: parent.id,
        edge: EdgeMeta::data(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_workflow_is_deterministic_and_well_formed() {
        let a = DemoWorkflow::canonical();
        let b = DemoWorkflow::canonical();
        // Same identities across two builds (precondition for replay).
        let ids_a: Vec<MoteId> = a.motes.iter().map(|w| w.mote.id).collect();
        let ids_b: Vec<MoteId> = b.motes.iter().map(|w| w.mote.id).collect();
        assert_eq!(ids_a, ids_b);
        assert_eq!(a.motes.len(), 6);

        // The shaper is present, is a topology shaper, and is READ-ONLY-NONDET
        // (R-14 forbids WORLD-MUTATING shapers).
        let shaper = a.motes.iter().find(|w| w.mote.id == a.shaper_id).unwrap();
        assert!(shaper.mote.def.is_topology_shaper);
        assert_eq!(shaper.mote.nd_class(), NdClass::ReadOnlyNondet);

        // The five nd_class / pattern roles are present.
        let kinds: Vec<(NdClass, EffectPattern)> = a
            .motes
            .iter()
            .map(|w| (w.mote.nd_class(), w.mote.def.effect_pattern))
            .collect();
        assert!(kinds.contains(&(NdClass::Pure, EffectPattern::IdempotentByConstruction)));
        assert!(kinds.contains(&(
            NdClass::ReadOnlyNondet,
            EffectPattern::IdempotentByConstruction
        )));
        assert!(kinds.contains(&(NdClass::WorldMutating, EffectPattern::StageThenCommit)));
        assert!(kinds.contains(&(NdClass::WorldMutating, EffectPattern::ValidateThenCommit)));

        // The critic targets M3 (the VTC Mote).
        let critic = a
            .motes
            .iter()
            .find(|w| w.mote.def.critic_for.is_some())
            .unwrap();
        assert_eq!(critic.mote.def.critic_for, Some(a.vtc_crash_target));
    }
}
