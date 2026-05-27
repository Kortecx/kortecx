//! [`TopologyMaterializer`] — the seam that fires on a Committed shaper
//! entry to read its [`kx_mote::TopologyDecision`] payload, derive each
//! child's [`kx_mote::MoteId`] per D49, and yield a [`crate::RegisterMote`] per
//! child for the projection to register.
//!
//! Production callers wire a [`DefaultTopologyMaterializer`] (which
//! composes a [`kx_content::ContentStore`] + a
//! [`crate::MoteDefRegistry`] + a [`crate::ChildResolver`]). Tests may pass a
//! lighter-weight stub.
//!
//! See `docs/design/decisions.md` §D48 + §D49 (private corpus) for the
//! load-bearing properties this seam must preserve: R49
//! reconstructibility, no-position-metadata, transitivity (P3),
//! re-run distinctness.

use std::sync::Arc;

use kx_content::{ContentRef, ContentStore};
use kx_mote::{
    derive_mote_id, EdgeKind, EdgeMeta, EffectPattern, GraphPosition, InputDataId, MoteDef,
    MoteDefHash, MoteId, NdClass, ParentRef, TopologyDecision,
};
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
/// topology shaper; if so, it fetches the [`TopologyDecision`] payload,
/// resolves each child's full [`MoteDef`] (D48), derives identity (D49),
/// and yields one [`RegisterMote`] per child.
///
/// MUST be deterministic in `(shaper_mote_id, shaper_def_hash,
/// shaper_result_ref)`. Replay-faithfulness rests on this.
pub trait TopologyMaterializer: Send + Sync {
    /// Return `Ok(Some(children))` if the Mote is a shaper whose
    /// `TopologyDecision` was successfully read + decoded; `Ok(None)`
    /// if the Mote is not a shaper (the common case — fast path); or
    /// `Err(_)` if the Mote IS a shaper but its payload could not be
    /// read or decoded.
    fn try_materialize(
        &self,
        shaper_mote_id: MoteId,
        shaper_def_hash: MoteDefHash,
        shaper_result_ref: ContentRef,
    ) -> Result<Option<Vec<RegisterMote>>, ProjectionError>;
}

/// Production [`TopologyMaterializer`] composing a content store, a
/// [`MoteDefRegistry`], and a [`ChildResolver`].
///
/// Generic over the three seams so callers can swap impls without
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
/// use kx_projection::{
///     DefaultTopologyMaterializer, InheritFromShaperResolver,
///     InMemoryMoteDefRegistry,
/// };
///
/// let store = Arc::new(InMemoryContentStore::new());
/// let registry = Arc::new(InMemoryMoteDefRegistry::new());
/// let materializer = DefaultTopologyMaterializer::new(
///     store,
///     registry,
///     InheritFromShaperResolver,
/// );
/// // pass `materializer` to `Projection::with_materializer(...)`.
/// # let _ = materializer;
/// ```
pub struct DefaultTopologyMaterializer<S, D, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    R: ChildResolver + 'static,
{
    store: Arc<S>,
    registry: Arc<D>,
    resolver: R,
}

impl<S, D, R> DefaultTopologyMaterializer<S, D, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    R: ChildResolver + 'static,
{
    /// Construct a materializer over the three seams.
    pub fn new(store: Arc<S>, registry: Arc<D>, resolver: R) -> Self {
        Self {
            store,
            registry,
            resolver,
        }
    }
}

impl<S, D, R> TopologyMaterializer for DefaultTopologyMaterializer<S, D, R>
where
    S: ContentStore + Send + Sync + 'static,
    D: MoteDefRegistry + 'static,
    R: ChildResolver + 'static,
{
    fn try_materialize(
        &self,
        shaper_mote_id: MoteId,
        shaper_def_hash: MoteDefHash,
        shaper_result_ref: ContentRef,
    ) -> Result<Option<Vec<RegisterMote>>, ProjectionError> {
        // 1. Look up shaper MoteDef. Unknown def → not a shaper from
        //    this materializer's perspective (silent skip, with a warn
        //    trace so misconfiguration is visible to operators).
        let Some(shaper_def) = self.registry.get(&shaper_def_hash) else {
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
            bincode::serde::decode_from_slice(payload.as_ref(), kx_mote::canonical_config())
                .map_err(|e| ProjectionError::TopologyDecodeFailed {
                    result_ref: shaper_result_ref,
                    details: format!("{e}"),
                })?;

        // 5. Resolve + derive identity for each child.
        let mut children = Vec::with_capacity(decision.children.len());
        for (index, descriptor) in decision.children.iter().enumerate() {
            children.push(derive_child_register_mote(
                shaper_mote_id,
                &shaper_def,
                shaper_result_ref,
                descriptor,
                index,
                &self.resolver,
            ));
        }
        Ok(Some(children))
    }
}

/// D49 identity derivation: compose a [`RegisterMote`] for one
/// shaper-spawned child from the shaper's identity facts + the
/// child's descriptor + the resolved child `MoteDef`.
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
    }
}

/// Convenience re-export of the pure D48/D49 identity primitives so
/// callers can derive child identity without going through the full
/// [`TopologyMaterializer`] (e.g. when testing or pre-computing).
///
/// Returns `(child_mote_id, child_def_hash)`. Same inputs → same outputs;
/// no I/O.
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
