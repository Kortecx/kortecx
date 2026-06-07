//! Coordinator-side derivation of a topology shaper's dispatchable children (PR-2b).
//!
//! When a shaper commits, its children must reach the worker. The projection knows a
//! child once it is registered, but the coordinator's **dispatch admission set**
//! ([`crate::state`]'s `Dispatch.defs`) is keyed only by SUBMITTED Motes — a child that
//! a client never submitted (the model invented it) would never reach `lease_ready`.
//!
//! This module rebuilds each child's FULL `(Mote, WarrantSpec)` from the SAME committed
//! `TopologyDecision` fact, reusing the SAME public kx-projection primitives the
//! projection's `DefaultTopologyMaterializer` uses — [`InheritFromShaperResolver`] for the
//! child `MoteDef` (D48, incl. the per-child `intent`), `Mote::new`'s `derive_mote_id` for
//! identity (D49), and [`intersect`] for the per-child warrant (SN-8 narrowing-only). So
//! the dispatch entry's `MoteId` provably equals the one the materializer registers (one
//! source of truth — asserted in tests). Pure + total over the inputs; the only I/O is one
//! content-store fetch + bincode decode of the committed decision. It runs identically on
//! the live commit-fold and on a recovery re-submit, so a crash between the shaper commit
//! and dispatch resumes correctly (R49: the decision is served from the fact, never
//! re-sampled).

use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_mote::{
    EdgeKind, EdgeMeta, GraphPosition, InputDataId, Mote, MoteDef, MoteId, ParentRef,
    TopologyDecision,
};
use kx_projection::{ChildResolver, InheritFromShaperResolver, RegisterMote};
use kx_warrant::{intersect, warrant_ref_of, RoleRegistry, WarrantSpec};
use smallvec::smallvec;

/// One materialized shaper child, ready for BOTH projection registration (so it enters
/// `ready_set`) and dispatch admission (so `lease_ready` can hand it to a worker).
pub(crate) struct ShaperChild {
    /// The lightweight registration the projection folds into its state.
    pub(crate) register: RegisterMote,
    /// The full Mote the worker re-runs (its `id` == `register.mote_id`).
    pub(crate) mote: Mote,
    /// The child's per-role-narrowed warrant.
    pub(crate) warrant: WarrantSpec,
}

/// Derive a committed topology shaper's dispatchable children from its committed
/// `TopologyDecision` fact. Returns `Ok(vec![])` for a non-shaper.
///
/// An error (decode / unknown-role / narrowing) is surfaced for the caller to log: by the
/// time the coordinator reaches here the shaper has already committed a `TopologyDecision`
/// that the shaper executor lowered + validated, so a failure means a provisioning
/// inconsistency (e.g. a role the coordinator's registry does not know) — the children
/// simply will not dispatch (degraded but safe; the shaper's commit stands).
pub(crate) fn derive_shaper_children(
    store: &LocalFsContentStore,
    role_registry: &dyn RoleRegistry,
    shaper_mote_id: MoteId,
    shaper_def: &MoteDef,
    shaper_result_ref: ContentRef,
    shaper_warrant: &WarrantSpec,
) -> Result<Vec<ShaperChild>, String> {
    if !shaper_def.is_topology_shaper {
        return Ok(Vec::new());
    }

    // Fetch + decode the committed decision (canonical bincode — the exact bytes the
    // shaper hashed into its `result_ref`, so the materializer reads the same thing).
    let payload = store
        .get(&shaper_result_ref)
        .map_err(|e| format!("fetch TopologyDecision {shaper_result_ref:?}: {e:?}"))?;
    let decision = TopologyDecision::decode(payload.as_ref())
        .map_err(|e| format!("decode TopologyDecision: {e}"))?;

    let resolver = InheritFromShaperResolver;
    let mut children = Vec::with_capacity(decision.children.len());
    for (index, descriptor) in decision.children.iter().enumerate() {
        // SN-8: the model names a ROLE; the registry maps it to a vetted warrant and
        // `intersect` narrows (never widens). An unknown role is a fail-closed error.
        let role = role_registry
            .resolve(&descriptor.role_id)
            .ok_or_else(|| format!("role {:?} not registered", descriptor.role_id))?;
        let child_warrant = intersect(shaper_warrant, &role)
            .map_err(|e| format!("narrowing child {index}: {e}"))?;
        let child_warrant_ref = warrant_ref_of(&child_warrant);

        // D48 — child MoteDef inherits the shaper's heavy axes + this child's `intent`.
        let child_def = resolver.resolve(shaper_def, descriptor);
        // D49 — identity from journal facts only (byte-identical to
        // `DefaultTopologyMaterializer::try_materialize` → `derive_child_register_mote`):
        // input_data_id = blake3(shaper.result_ref); graph_position = shaper_id ‖ index_le.
        // `ContentRef::of` IS blake3-of-bytes (kx-content), so the coordinator reuses it
        // rather than taking a direct `blake3` dep (D111: Cargo.lock unchanged).
        let child_input_data_id =
            InputDataId::from_bytes(*ContentRef::of(shaper_result_ref.as_bytes()).as_bytes());
        let mut gp = Vec::with_capacity(36);
        gp.extend_from_slice(shaper_mote_id.as_bytes());
        // `index as u32` byte-for-byte matches `DefaultTopologyMaterializer`'s graph
        // position (kx-projection) — the cast MUST be identical for the child `MoteId`s to
        // match; a child count never approaches u32::MAX (it is `LoopBudget`-bounded).
        #[allow(clippy::cast_possible_truncation)]
        let index_le = (index as u32).to_le_bytes();
        gp.extend_from_slice(&index_le);
        let child_graph_position = GraphPosition(gp);
        // Single Control edge from the shaper (cascade by default), matching the
        // materializer.
        let parents = smallvec![ParentRef {
            parent_id: shaper_mote_id,
            edge: EdgeMeta {
                kind: EdgeKind::Control,
                non_cascade: false,
            },
        }];

        let mote = Mote::new(
            child_def.clone(),
            child_input_data_id,
            child_graph_position,
            parents.clone(),
        );
        let register = RegisterMote {
            mote_id: mote.id,
            nd_class: child_def.nd_class,
            effect_pattern: child_def.effect_pattern,
            critic_for: None,
            is_topology_shaper: false,
            parents,
            warrant_ref: child_warrant_ref,
        };
        children.push(ShaperChild {
            register,
            mote,
            warrant: child_warrant,
        });
    }
    Ok(children)
}
