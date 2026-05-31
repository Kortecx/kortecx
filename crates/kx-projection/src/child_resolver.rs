//! D48 child resolver seam.
//!
//! [`ChildResolver`] composes the full child `MoteDef` from the shaper's
//! `MoteDef` + the descriptor's per-child overrides. [`InheritFromShaperResolver`]
//! is the OSS-default trivial impl: inherits the heavy axes (model, prompt,
//! tools, config) from the shaper; descriptor wins on the per-child axes
//! (logic_ref, nd_class, effect_pattern); a child cannot be born as a
//! critic or a shaper (those would have to be explicit workflow-author
//! choices).
//!
//! See `docs/design/decisions.md` §D48 (private corpus) for the full
//! cloud forward-note: cloud impls that resolve per-role MoteDefs must
//! commit the resolved MoteDef to the journal (same shape as
//! `TopologyDecision` itself, per D37) so replay sees the same
//! resolution. OSS inheritance is replay-trivial; cloud registry-backed
//! resolution is NOT.

use kx_mote::{ChildDescriptor, MoteDef, MOTE_DEF_SCHEMA_VERSION};

/// The D48 seam: compose a full child `MoteDef` from the shaper's
/// `MoteDef` + the child's descriptor.
///
/// MUST be pure / total / deterministic. Same `(shaper_def, descriptor)`
/// in → byte-identical `MoteDef` out. Replay-safety rests on this.
pub trait ChildResolver: Send + Sync {
    /// Resolve the child's full `MoteDef`. The returned value is
    /// `MoteDef::hash()`'d to produce `child_mote_def_hash`, which
    /// then feeds [`kx_mote::derive_mote_id`] together with the
    /// child's D49 `input_data_id` and `graph_position`.
    fn resolve(&self, shaper_def: &MoteDef, descriptor: &ChildDescriptor) -> MoteDef;
}

/// OSS-default resolver: inherit from the shaper, override per the
/// descriptor.
///
/// - Inherited from shaper: `model_id`, `prompt_template_hash`,
///   `tool_contract`, `config_subset`.
/// - Overridden by descriptor: `logic_ref`, `nd_class`, `effect_pattern`.
/// - Hardcoded for children: `critic_for = None`, `is_topology_shaper = false`.
/// - Always current: `schema_version`.
///
/// **`is_topology_shaper = false` is structural** — children cannot be
/// born as shapers via inheritance. Multi-level shaper hierarchies
/// remain expressible at workflow-author-declared scope (a child Mote
/// can be resubmitted as a shaper in a later workflow), but the
/// resolver itself never births a shaper child.
///
/// # Examples
///
/// ```
/// use kx_mote::{ChildDescriptor, EffectPattern, LogicRef, MoteDef, NdClass, RoleId};
/// use kx_projection::{ChildResolver, InheritFromShaperResolver};
/// use std::collections::BTreeMap;
///
/// let shaper = MoteDef {
///     logic_ref: LogicRef([1u8; 32]),
///     model_id: kx_mote::ModelId("test-model".into()),
///     prompt_template_hash: kx_mote::PromptTemplateHash([3u8; 32]),
///     tool_contract: BTreeMap::new(),
///     nd_class: NdClass::ReadOnlyNondet,
///     config_subset: BTreeMap::new(),
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     critic_for: None,
///     is_topology_shaper: true,
///     inference_params: kx_mote::InferenceParams::default(),
///     critic_check: None,
///     schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
/// };
/// let descriptor = ChildDescriptor {
///     role_id: RoleId("worker".into()),
///     logic_ref: LogicRef([42u8; 32]),
///     nd_class: NdClass::Pure,
///     effect_pattern: EffectPattern::IdempotentByConstruction,
/// };
/// let child = InheritFromShaperResolver.resolve(&shaper, &descriptor);
/// // Inherited:
/// assert_eq!(&child.model_id, &shaper.model_id);
/// assert_eq!(child.prompt_template_hash, shaper.prompt_template_hash);
/// // Overridden:
/// assert_eq!(child.logic_ref, descriptor.logic_ref);
/// assert_eq!(child.nd_class, descriptor.nd_class);
/// // Hardcoded:
/// assert!(child.critic_for.is_none());
/// assert!(!child.is_topology_shaper);
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct InheritFromShaperResolver;

impl ChildResolver for InheritFromShaperResolver {
    fn resolve(&self, shaper: &MoteDef, d: &ChildDescriptor) -> MoteDef {
        MoteDef {
            critic_check: None,
            // Inherited from shaper (the heavy axes — D48 + D50).
            model_id: shaper.model_id.clone(),
            prompt_template_hash: shaper.prompt_template_hash,
            tool_contract: shaper.tool_contract.clone(),
            config_subset: shaper.config_subset.clone(),
            inference_params: shaper.inference_params.clone(),
            // Hardcoded for materialized children — see D48 anti-patterns.
            critic_for: None,
            is_topology_shaper: false,
            // Always the current schema for newly-materialized Motes.
            schema_version: MOTE_DEF_SCHEMA_VERSION,
            // Descriptor-overridden (the per-child axes).
            logic_ref: d.logic_ref,
            nd_class: d.nd_class,
            effect_pattern: d.effect_pattern,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_mote::{EffectPattern, LogicRef, NdClass, PromptTemplateHash, RoleId};
    use std::collections::BTreeMap;

    fn shaper_def() -> MoteDef {
        let mut tools = BTreeMap::new();
        tools.insert(
            kx_mote::ToolName("fs-read".into()),
            kx_mote::ToolVersion("1.0".into()),
        );
        let mut cfg = BTreeMap::new();
        cfg.insert(
            kx_mote::ConfigKey("temperature".into()),
            kx_mote::ConfigVal("0.0".into()),
        );
        MoteDef {
            critic_check: None,
            logic_ref: LogicRef([1u8; 32]),
            model_id: kx_mote::ModelId("planner-v1".into()),
            prompt_template_hash: PromptTemplateHash([3u8; 32]),
            tool_contract: tools,
            nd_class: NdClass::ReadOnlyNondet,
            config_subset: cfg,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: true,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    fn descriptor(seed: u8, nd: NdClass, ep: EffectPattern) -> ChildDescriptor {
        ChildDescriptor {
            role_id: RoleId(format!("role-{seed}")),
            logic_ref: LogicRef([seed; 32]),
            nd_class: nd,
            effect_pattern: ep,
        }
    }

    #[test]
    fn child_inherits_heavy_axes_from_shaper() {
        let shaper = shaper_def();
        let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        assert_eq!(child.model_id, shaper.model_id);
        assert_eq!(child.prompt_template_hash, shaper.prompt_template_hash);
        assert_eq!(child.tool_contract, shaper.tool_contract);
        assert_eq!(child.config_subset, shaper.config_subset);
    }

    #[test]
    fn child_descriptor_overrides_per_child_axes() {
        let shaper = shaper_def();
        let d = descriptor(7, NdClass::Pure, EffectPattern::StageThenCommit);
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        assert_eq!(child.logic_ref, d.logic_ref);
        assert_eq!(child.nd_class, d.nd_class);
        assert_eq!(child.effect_pattern, d.effect_pattern);
        // Confirm descriptor really did change them away from shaper's:
        assert_ne!(child.logic_ref, shaper.logic_ref);
        assert_ne!(child.nd_class, shaper.nd_class);
        assert_ne!(child.effect_pattern, shaper.effect_pattern);
    }

    #[test]
    fn child_is_never_a_critic_or_shaper_by_inheritance() {
        let mut shaper = shaper_def();
        // Even if the shaper itself were marked as a critic (which R-8
        // refuses at submission), the resolver must NOT propagate that.
        shaper.critic_for = Some(kx_mote::MoteId::from_bytes([99u8; 32]));
        shaper.is_topology_shaper = true;
        let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        assert!(child.critic_for.is_none());
        assert!(!child.is_topology_shaper);
    }

    #[test]
    fn resolver_is_deterministic_same_inputs_same_output() {
        let shaper = shaper_def();
        let d = descriptor(11, NdClass::WorldMutating, EffectPattern::StageThenCommit);
        let a = InheritFromShaperResolver.resolve(&shaper, &d);
        let b = InheritFromShaperResolver.resolve(&shaper, &d);
        assert_eq!(a, b);
        // Hashes also bit-identical:
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn descriptor_byte_change_changes_child_hash() {
        // The P3 transitivity property at the resolver layer: a one-axis
        // change in the descriptor MUST change the resolved MoteDef's hash.
        // (D49 P3 also tests this end-to-end via the materializer; this is
        // the resolver-layer slice.)
        let shaper = shaper_def();
        let a = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor(1, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        );
        // Change logic_ref:
        let b = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor(2, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        );
        assert_ne!(a.hash(), b.hash());
        // Change nd_class:
        let c = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor(
                1,
                NdClass::ReadOnlyNondet,
                EffectPattern::IdempotentByConstruction,
            ),
        );
        assert_ne!(a.hash(), c.hash());
        // Change effect_pattern:
        let dd = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor(1, NdClass::Pure, EffectPattern::StageThenCommit),
        );
        assert_ne!(a.hash(), dd.hash());
    }
}
