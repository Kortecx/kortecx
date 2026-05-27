//! [`TopologyMaterializer`] — the seam that fires on a Committed shaper
//! entry to read its [`kx_mote::TopologyDecision`] payload, derive each
//! child's [`kx_mote::MoteId`] per D49, narrow each child's warrant per
//! D30 + `topology.md` §13 KG-1-close (PR 11.5), and yield a
//! [`crate::RegisterMote`] per child for the projection to register.
//!
//! Production callers wire a [`DefaultTopologyMaterializer`] (which
//! composes a [`kx_content::ContentStore`], a [`crate::MoteDefRegistry`],
//! a [`crate::ChildResolver`], and a [`kx_warrant::RoleRegistry`]). Tests
//! may pass a lighter-weight stub.
//!
//! See `docs/design/decisions.md` §D30 + §D48 + §D49 +
//! `docs/design/topology.md` §13 KG-1 (private corpus) for the
//! load-bearing properties this seam must preserve: R49
//! reconstructibility, no-position-metadata, transitivity (P3),
//! re-run distinctness, AND per-role warrant narrowing.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore};
use kx_mote::{
    canonical_config, derive_mote_id, EdgeKind, EdgeMeta, EffectPattern, GraphPosition,
    InputDataId, MoteDef, MoteDefHash, MoteId, NdClass, ParentRef, TopologyDecision,
};
use kx_warrant::{intersect, warrant_ref_of, RoleRegistry, WarrantSpec};
use smallvec::smallvec;
use tracing::warn;

use crate::child_resolver::ChildResolver;
use crate::errors::ProjectionError;
use crate::mote_def_registry::MoteDefRegistry;
use crate::register::RegisterMote;

/// The materializer seam.
///
/// Invoked by [`crate::Projection::fold`] on every `Committed` journal
/// entry. The materializer decides — by looking up the Mote's
/// [`MoteDef`] via [`MoteDefRegistry`] — whether the Mote is a
/// topology shaper; if so, it fetches the [`TopologyDecision`] payload
/// together with the shaper's [`WarrantSpec`], resolves each child's
/// full [`MoteDef`] (D48), narrows the child's warrant per D30 using a
/// [`RoleRegistry`] (PR 11.5 / KG-1-close), derives identity (D49),
/// and yields one [`RegisterMote`] per child.
///
/// MUST be deterministic in `(shaper_mote_id, shaper_def_hash,
/// shaper_result_ref, shaper_warrant_ref)`. Replay-faithfulness rests
/// on this.
pub trait TopologyMaterializer: Send + Sync {
    /// Return `Ok(Some(children))` if the Mote is a shaper whose
    /// `TopologyDecision` was successfully read + decoded; `Ok(None)`
    /// if the Mote is not a shaper (the common case — fast path); or
    /// `Err(_)` if the Mote IS a shaper but its payload could not be
    /// read or decoded, or some descriptor's role is not registered, or
    /// the role's spec attempts to widen the shaper's warrant.
    fn try_materialize(
        &self,
        shaper_mote_id: MoteId,
        shaper_def_hash: MoteDefHash,
        shaper_result_ref: ContentRef,
        shaper_warrant_ref: ContentRef,
    ) -> Result<Option<Vec<RegisterMote>>, ProjectionError>;
}

/// Production [`TopologyMaterializer`] composing a content store, a
/// [`MoteDefRegistry`], a [`ChildResolver`], and a [`RoleRegistry`].
///
/// Generic over the four seams so callers can swap impls without
/// touching the projection. The content store is held behind `Arc`
/// (because [`ContentStore`]'s associated `Payload` type makes
/// `dyn ContentStore` impractical — see the `BodyResolver` pattern in
/// `kx-executor` for the established precedent).
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use kx_content::InMemoryContentStore;
/// use kx_warrant::InMemoryRoleRegistry;
/// use kx_projection::{
///     DefaultTopologyMaterializer, InheritFromShaperResolver,
///     InMemoryMoteDefRegistry,
/// };
///
/// let store = Arc::new(InMemoryContentStore::new());
/// let def_registry = Arc::new(InMemoryMoteDefRegistry::new());
/// let role_registry = Arc::new(InMemoryRoleRegistry::new());
/// let materializer = DefaultTopologyMaterializer::new(
///     store,
///     def_registry,
///     role_registry,
///     InheritFromShaperResolver,
/// );
/// // pass `materializer` to `Projection::with_materializer(...)`.
/// # let _ = materializer;
/// ```
pub struct DefaultTopologyMaterializer<S, D, Reg, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    Reg: RoleRegistry + 'static,
    R: ChildResolver + 'static,
{
    store: Arc<S>,
    def_registry: Arc<D>,
    role_registry: Arc<Reg>,
    resolver: R,
}

impl<S, D, Reg, R> DefaultTopologyMaterializer<S, D, Reg, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    Reg: RoleRegistry + 'static,
    R: ChildResolver + 'static,
{
    /// Construct a materializer over the four seams.
    pub fn new(store: Arc<S>, def_registry: Arc<D>, role_registry: Arc<Reg>, resolver: R) -> Self {
        Self {
            store,
            def_registry,
            role_registry,
            resolver,
        }
    }
}

impl<S, D, Reg, R> TopologyMaterializer for DefaultTopologyMaterializer<S, D, Reg, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    Reg: RoleRegistry + 'static,
    R: ChildResolver + 'static,
{
    fn try_materialize(
        &self,
        shaper_mote_id: MoteId,
        shaper_def_hash: MoteDefHash,
        shaper_result_ref: ContentRef,
        shaper_warrant_ref: ContentRef,
    ) -> Result<Option<Vec<RegisterMote>>, ProjectionError> {
        // 1. Look up shaper MoteDef. Unknown def → not a shaper from
        //    this materializer's perspective (silent skip, with a warn
        //    trace so misconfiguration is visible to operators).
        let Some(shaper_def) = self.def_registry.get(&shaper_def_hash) else {
            warn!(
                shaper_mote_id = ?shaper_mote_id,
                shaper_def_hash = ?shaper_def_hash,
                "materializer: shaper MoteDef not registered; skipping (workflow author MUST register every referenced MoteDef)"
            );
            return Ok(None);
        };

        // 2. Fast path: not a shaper.
        if !shaper_def.is_topology_shaper {
            return Ok(None);
        }

        // 3. Fetch the TopologyDecision payload from the content store.
        //    Per D39 §c, the payload is byte-immutable at this ref.
        let payload =
            self.store
                .get(&shaper_result_ref)
                .map_err(|e| ProjectionError::ContentStoreFetch {
                    result_ref: shaper_result_ref,
                    details: format!("{e:?}"),
                })?;

        // 4. Decode as TopologyDecision via canonical bincode (the same
        //    encoding the shaper used to compute its result_ref hash).
        let (decision, _consumed): (TopologyDecision, usize) =
            bincode::serde::decode_from_slice(payload.as_ref(), canonical_config()).map_err(
                |e| ProjectionError::TopologyDecodeFailed {
                    result_ref: shaper_result_ref,
                    details: format!("{e}"),
                },
            )?;

        // 5. **PR 11.5 / KG-1-close.** Fetch + decode the shaper's
        //    WarrantSpec. Required for the per-child intersect call.
        let warrant_payload = self.store.get(&shaper_warrant_ref).map_err(|e| {
            ProjectionError::WarrantStoreFetch {
                warrant_ref: shaper_warrant_ref,
                details: format!("{e:?}"),
            }
        })?;
        let (shaper_warrant, _consumed): (WarrantSpec, usize) =
            bincode::serde::decode_from_slice(warrant_payload.as_ref(), canonical_config())
                .map_err(|e| ProjectionError::WarrantDecodeFailed {
                    warrant_ref: shaper_warrant_ref,
                    details: format!("{e}"),
                })?;

        // 6. Resolve + narrow + derive identity for each child.
        let mut children = Vec::with_capacity(decision.children.len());
        for (index, descriptor) in decision.children.iter().enumerate() {
            // 6a. **PR 11.5 / KG-1-close.** Resolve the descriptor's
            //     RoleId → Role via the role registry. Missing role is
            //     a typed error — the materializer refuses to silently
            //     widen.
            let role = self
                .role_registry
                .resolve(&descriptor.role_id)
                .ok_or_else(|| ProjectionError::RoleNotRegistered {
                    role_id: descriptor.role_id.clone(),
                    descriptor_index: index,
                })?;

            // 6b. **PR 11.5 / KG-1-close.** D30 monotonic narrowing.
            //     Replaces PR 11's verbatim ref-copy with the actual
            //     intersection. NarrowingError (widening attempt /
            //     syscall mismatch / invalid model route) is a fold
            //     error — the workflow author sees the offending axis.
            let child_warrant = intersect(&shaper_warrant, &role).map_err(|e| {
                ProjectionError::NarrowingFailed {
                    descriptor_index: index,
                    details: format!("{e}"),
                }
            })?;
            let child_warrant_ref = warrant_ref_of(&child_warrant);

            children.push(derive_child_register_mote(
                shaper_mote_id,
                &shaper_def,
                shaper_result_ref,
                descriptor,
                index,
                &self.resolver,
                child_warrant_ref,
            ));
        }
        Ok(Some(children))
    }
}

/// D49 identity derivation + KG-1-close warrant_ref population: compose
/// a [`RegisterMote`] for one shaper-spawned child from the shaper's
/// identity facts + the child's descriptor + the resolved child
/// `MoteDef` + the per-role-narrowed `warrant_ref`.
///
/// This is the **load-bearing function** for R49. Made `pub(crate)` so
/// the [`DefaultTopologyMaterializer`] and tests can both call it
/// without going through trait dispatch.
///
/// Identity formula (D49 §"Chosen"):
///
/// ```text
/// child_mote_id = blake3(
///     child_mote_def_hash       // from D48's resolver
///     ‖ child_input_data_id     // = blake3(shaper.committed.result_ref.bytes)
///     ‖ child_graph_position    // = shaper.MoteId.bytes ‖ child_index_u32_le
/// )
/// ```
pub(crate) fn derive_child_register_mote(
    shaper_mote_id: MoteId,
    shaper_def: &MoteDef,
    shaper_result_ref: ContentRef,
    descriptor: &kx_mote::ChildDescriptor,
    index: usize,
    resolver: &dyn ChildResolver,
    child_warrant_ref: ContentRef,
) -> RegisterMote {
    // D48 — compose child's full MoteDef.
    let child_def = resolver.resolve(shaper_def, descriptor);
    let child_def_hash = child_def.hash();

    // D49 — derive identity from journal facts only.
    // input_data_id = blake3(shaper.committed.result_ref.bytes)
    let child_input_data_id =
        InputDataId::from_bytes(*blake3::hash(shaper_result_ref.as_bytes()).as_bytes());

    // graph_position = shaper.MoteId.bytes ‖ child_index_u32_le
    let mut gp_bytes = Vec::with_capacity(36);
    gp_bytes.extend_from_slice(shaper_mote_id.as_bytes());
    gp_bytes.extend_from_slice(&(index as u32).to_le_bytes());
    let child_graph_position = GraphPosition(gp_bytes);

    let child_mote_id =
        derive_mote_id(&child_def_hash, &child_input_data_id, &child_graph_position);

    // Single Control edge from shaper (per the corpus-freeze
    // decision; multi-parent shaper-spawned children deferred). Per
    // `control-edge-cascade-default.md`, Control edges cascade by
    // default — `non_cascade = false`.
    let parents = smallvec![ParentRef {
        parent_id: shaper_mote_id,
        edge: EdgeMeta {
            kind: EdgeKind::Control,
            non_cascade: false,
        },
    }];

    RegisterMote {
        mote_id: child_mote_id,
        nd_class: child_def.nd_class,
        effect_pattern: child_def.effect_pattern,
        critic_for: None,
        is_topology_shaper: false,
        parents,
        warrant_ref: child_warrant_ref,
    }
}

/// Convenience re-export of the pure D48/D49 identity primitives so
/// callers can derive child identity without going through the full
/// [`TopologyMaterializer`] (e.g. when testing or pre-computing).
///
/// Returns `(child_mote_id, child_def_hash, child_nd_class,
/// child_effect_pattern)`. Same inputs → same outputs; no I/O.
///
/// **Does NOT compute the child's narrowed warrant** — that requires a
/// `RoleRegistry` + `intersect` call (see
/// [`DefaultTopologyMaterializer::try_materialize`]). This helper
/// exists for tests that only need to derive identity.
#[must_use]
pub fn derive_child_identity(
    shaper_mote_id: MoteId,
    shaper_def: &MoteDef,
    shaper_result_ref: ContentRef,
    descriptor: &kx_mote::ChildDescriptor,
    index: usize,
    resolver: &dyn ChildResolver,
) -> (MoteId, MoteDefHash, NdClass, EffectPattern) {
    let child_def = resolver.resolve(shaper_def, descriptor);
    let child_def_hash = child_def.hash();
    let child_input_data_id =
        InputDataId::from_bytes(*blake3::hash(shaper_result_ref.as_bytes()).as_bytes());
    let mut gp_bytes = Vec::with_capacity(36);
    gp_bytes.extend_from_slice(shaper_mote_id.as_bytes());
    gp_bytes.extend_from_slice(&(index as u32).to_le_bytes());
    let child_graph_position = GraphPosition(gp_bytes);
    let child_mote_id =
        derive_mote_id(&child_def_hash, &child_input_data_id, &child_graph_position);
    (
        child_mote_id,
        child_def_hash,
        child_def.nd_class,
        child_def.effect_pattern,
    )
}
