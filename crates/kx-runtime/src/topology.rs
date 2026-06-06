//! The demo's topology shaper — the "runtime-discovered topology decision"
//! the P1 exit gate requires to survive replay.
//!
//! The shaper `S` (READ-ONLY-NONDET, `is_topology_shaper`) commits a
//! [`TopologyDecision`] that spawns two PURE worker children. The projection's
//! [`TopologyMaterializer`] re-derives those children — deterministically and
//! identically — every time the shaper's `Committed` entry is folded, including
//! on a fresh-process replay (R49). This module owns the demo's decision, the
//! materializer wiring, and the engine-side re-derivation of *runnable* child
//! Motes (identity-matched to what the materializer registers).

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore};
use kx_mote::{
    canonical_config, ChildDescriptor, EdgeMeta, EffectPattern, GraphPosition, InputDataId,
    LogicRef, Mote, MoteDef, NdClass, ParentRef, RoleId, TopologyDecision,
};
use kx_projection::{
    ChildResolver, DefaultTopologyMaterializer, InMemoryMoteDefRegistry, InheritFromShaperResolver,
    Snapshot, TopologyMaterializer,
};
use kx_warrant::{InMemoryRoleRegistry, Role, WarrantSpec};
use smallvec::SmallVec;
use thiserror::Error;

use crate::error::RuntimeError;
use crate::workflow::WorkflowMote;

/// The PR-2 (F-4) seam: a **model** computes a topology shaper's
/// [`TopologyDecision`] at dispatch time, replacing the hardcoded
/// [`demo_topology_decision`]. This is "the model drives the loop": the shaper's
/// committed `result_ref` is the *model's* lowered decision, a captured fact the
/// projection materializer re-derives children from on every fold (incl. replay).
///
/// Object-safe (`&dyn TopologyProvider`) and `Send + Sync` so the engine can hold
/// `Option<&dyn TopologyProvider>`. It carries **no** model/inference type — the
/// concrete provider lives in `kx-model-harness` (the dependency arrow points
/// down; `kx-runtime` never depends on the harness/inference). It receives only
/// what the engine already holds at the shaper-dispatch point: the shaper `Mote`,
/// its `WarrantSpec`, and the current committed-state [`Snapshot`].
///
/// SN-8: the provider **proposes** (the model output is untrusted — the concrete
/// impl decodes it fail-closed and lowers role names through *vetted recipes*);
/// the **runtime decides** authority — each materialized child's warrant is
/// `intersect(shaper.warrant, role)` (narrowing-only) downstream. A fail-closed
/// provider (malformed / oversized / un-grantable proposal, or an exhausted
/// budget) returns [`TopologyProviderError`]; the caller dead-letters the shaper
/// (composing with the PR-1 failure policy), never panics, never re-runs it.
///
/// PR-2 invokes this **eagerly** in the harness (the single parentless shaper is
/// computed once, before the run, then served as the shaper's effect). PR-3
/// (re-plan) will have the engine invoke it **lazily** per round against the live
/// snapshot — the same seam, no signature change.
pub trait TopologyProvider: Send + Sync {
    /// Compute this shaper's [`TopologyDecision`] from the model's proposal,
    /// assembled against `snapshot`. The caller encodes the returned decision
    /// (canonical bincode) and commits it as the shaper's `result_ref`, so a cold
    /// re-fold re-materializes byte-identical children (R49). Returns
    /// [`TopologyProviderError`] for any fail-closed refusal.
    fn decide(
        &self,
        shaper: &Mote,
        shaper_warrant: &WarrantSpec,
        snapshot: &Snapshot,
    ) -> Result<TopologyDecision, TopologyProviderError>;

    /// Build the topology materializer the runtime folds through while THIS
    /// provider drives the loop. The demo's [`build_materializer`] only resolves
    /// the `demo-worker` role; a model-driven decision proposes arbitrary roles,
    /// so the provider supplies a materializer whose role registry resolves every
    /// role its [`decide`](Self::decide) can emit (still `intersect(shaper.warrant,
    /// role)`-narrowing — SN-8). The runtime calls this once, at projection build,
    /// in place of `build_materializer` whenever a provider is present; the
    /// returned materializer MUST read the SAME content store the run commits to
    /// (so the committed decision + warrant bytes resolve).
    fn materializer(
        &self,
        shaper_def: &MoteDef,
        shaper_warrant: &WarrantSpec,
    ) -> Box<dyn TopologyMaterializer>;
}

/// Why a [`TopologyProvider`] could not produce a decision. Opaque to the engine
/// (a `String` payload keeps `kx-runtime` free of a `kx-planner`/inference dep —
/// the harness formats its `PlanError` / backend error into it). The caller
/// dead-letters the shaper on this; it is never a panic and never a silent skip.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("topology provider could not produce a decision: {0}")]
pub struct TopologyProviderError(pub String);

/// Decode the canonical-bincode bytes a topology shaper committed as its
/// `result_ref` back into a [`TopologyDecision`] — the exact inverse of
/// [`encode_topology_decision`], and the identical decode the
/// `DefaultTopologyMaterializer` performs. Used by the engine's fact-driven child
/// derivation so its runnable children are provably the SAME set the materializer
/// registers (both decode one committed fact — a single source of truth).
pub fn decode_topology_decision(bytes: &[u8]) -> Result<TopologyDecision, RuntimeError> {
    let (td, _) =
        bincode::serde::decode_from_slice::<TopologyDecision, _>(bytes, canonical_config())
            .map_err(|e| RuntimeError::Decode(format!("topology decision: {e}")))?;
    Ok(td)
}

/// The `RoleId` every demo worker child takes.
pub const DEMO_WORKER_ROLE: &str = "demo-worker";
/// How many worker children the demo shaper spawns.
pub const DEMO_WORKER_COUNT: usize = 2;

/// The demo shaper's topology decision: `DEMO_WORKER_COUNT` PURE workers, each
/// with a distinct `logic_ref` (so they are distinct Motes), all taking the
/// `demo-worker` role.
#[must_use]
pub fn demo_topology_decision() -> TopologyDecision {
    let children = (0..DEMO_WORKER_COUNT)
        .map(|i| {
            let tag = 0xB0_u8.wrapping_add(u8::try_from(i).unwrap_or(0));
            ChildDescriptor {
                role_id: RoleId(DEMO_WORKER_ROLE.into()),
                logic_ref: LogicRef::from_bytes([tag; 32]),
                nd_class: NdClass::Pure,
                effect_pattern: EffectPattern::IdempotentByConstruction,
            }
        })
        .collect();
    TopologyDecision { children }
}

/// Canonical bincode of a `TopologyDecision` — the exact bytes the materializer
/// decodes (and whose hash is the shaper's committed `result_ref`).
pub fn encode_topology_decision(td: &TopologyDecision) -> Result<Vec<u8>, RuntimeError> {
    bincode::serde::encode_to_vec(td, canonical_config())
        .map_err(|e| RuntimeError::Encode(format!("topology decision: {e}")))
}

/// Canonical bincode of a `WarrantSpec` — the bytes the materializer fetches by
/// the shaper's `warrant_ref` to perform per-child narrowing.
pub fn encode_warrant(warrant: &WarrantSpec) -> Result<Vec<u8>, RuntimeError> {
    bincode::serde::encode_to_vec(warrant, canonical_config())
        .map_err(|e| RuntimeError::Encode(format!("warrant: {e}")))
}

/// The role registry the materializer resolves child `role_id`s against. The
/// `demo-worker` role's spec is the (permissive) shaper warrant, so
/// `intersect(shaper_warrant, role.spec)` is a no-op and the child warrant
/// equals the shaper's — keeping the demo's narrowing trivially deterministic.
#[must_use]
pub fn demo_role_registry(shaper_warrant: &WarrantSpec) -> Arc<InMemoryRoleRegistry> {
    let reg = InMemoryRoleRegistry::new();
    reg.register(
        RoleId(DEMO_WORKER_ROLE.into()),
        Role {
            name: DEMO_WORKER_ROLE.into(),
            version: 1,
            spec: shaper_warrant.clone(),
            description: "demo worker child".into(),
        },
    );
    Arc::new(reg)
}

/// Build the topology materializer the engine's projection folds through:
/// content store (for the decision + warrant payloads) + a def registry
/// carrying the shaper's `MoteDef` + the role registry + the inherit resolver.
#[must_use]
pub fn build_materializer<S>(
    store: Arc<S>,
    shaper_def: &MoteDef,
    shaper_warrant: &WarrantSpec,
) -> Box<dyn TopologyMaterializer>
where
    S: ContentStore + Send + Sync + 'static,
{
    let def_registry = InMemoryMoteDefRegistry::new();
    def_registry.register(shaper_def.clone());
    let role_registry = demo_role_registry(shaper_warrant);
    Box::new(DefaultTopologyMaterializer::new(
        store,
        Arc::new(def_registry),
        role_registry,
        InheritFromShaperResolver,
    ))
}

/// Re-derive the shaper's children as *runnable* [`WorkflowMote`]s, identity-
/// matched to what the materializer registers (same `derive_mote_id` inputs:
/// child def hash ‖ `blake3(shaper_result_ref)` ‖ shaper_id‖index). The engine
/// appends these to its runnable set once the shaper commits, so the children
/// actually execute (not merely materialize).
#[must_use]
pub fn derive_child_motes(
    shaper: &Mote,
    shaper_result_ref: ContentRef,
    td: &TopologyDecision,
    child_warrant: &WarrantSpec,
    capability: &kx_mote::ToolName,
) -> Vec<WorkflowMote> {
    let resolver = InheritFromShaperResolver;
    let child_input_data_id =
        InputDataId::from_bytes(*blake3::hash(shaper_result_ref.as_bytes()).as_bytes());

    td.children
        .iter()
        .enumerate()
        .map(|(index, descriptor)| {
            let child_def = resolver.resolve(&shaper.def, descriptor);

            let mut gp_bytes = Vec::with_capacity(36);
            gp_bytes.extend_from_slice(shaper.id.as_bytes());
            gp_bytes.extend_from_slice(&u32::try_from(index).unwrap_or(0).to_le_bytes());

            let parents: SmallVec<[ParentRef; 4]> = std::iter::once(ParentRef {
                parent_id: shaper.id,
                edge: EdgeMeta::control(),
            })
            .collect();

            let child = Mote::new(
                child_def,
                child_input_data_id,
                GraphPosition(gp_bytes),
                parents,
            );
            WorkflowMote {
                mote: child,
                warrant: child_warrant.clone(),
                capability: capability.clone(),
            }
        })
        .collect()
}
