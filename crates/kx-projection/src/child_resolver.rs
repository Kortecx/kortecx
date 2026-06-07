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

use kx_mote::{ChildDescriptor, ConfigKey, MoteDef, MOTE_DEF_SCHEMA_VERSION, PROMPT_KEY};

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
///   `tool_contract`, `config_subset` (with the prompt key overridden when the
///   descriptor carries a non-empty `intent` — see below).
/// - Overridden by descriptor: `logic_ref`, `nd_class`, `effect_pattern`, and —
///   when non-empty — the child's `config_subset[PROMPT_KEY]` from `intent`.
/// - Hardcoded for children: `critic_for = None`, `is_topology_shaper = false`.
/// - Always current: `schema_version`.
///
/// **Per-child intent (override-when-nonempty).** A descriptor's `intent`, when
/// non-empty, replaces the inherited prompt so a corrective child runs its own
/// instruction; an empty `intent` is a no-op (byte-identical to the pre-intent
/// resolver). `intent` only ever writes the prompt key — never an authority axis
/// (warrant narrowing happens upstream via `intersect`, SN-8 unaffected).
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
///     intent: kx_mote::ConfigVal(Vec::new()),
/// };
/// let child = InheritFromShaperResolver.resolve(&shaper, &descriptor);
/// // Inherited (empty intent ⇒ config_subset inherited verbatim):
/// assert_eq!(&child.model_id, &shaper.model_id);
/// assert_eq!(child.prompt_template_hash, shaper.prompt_template_hash);
/// assert_eq!(child.config_subset, shaper.config_subset);
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
        // Inherit the shaper's `config_subset` (the heavy axes), then let a
        // NON-EMPTY per-child `intent` OVERRIDE the prompt key, so a corrective
        // child runs ITS OWN instruction rather than re-running the shaper's
        // planning prompt. An EMPTY `intent` is a no-op ⇒ the child inherits the
        // shaper's prompt verbatim, so the resolved `MoteDef` is byte-identical
        // to the pre-intent behavior (and the canonical demo's empty-intent
        // children keep their exact `MoteId`s). Pure / total / deterministic:
        // same `(shaper, descriptor)` in ⇒ byte-identical `MoteDef` out, on the
        // live path AND on cold-refold (R49), because `intent` rides in the
        // committed `TopologyDecision` the materializer re-reads.
        let mut config_subset = shaper.config_subset.clone();
        if !d.intent.0.is_empty() {
            config_subset.insert(ConfigKey(PROMPT_KEY.to_string()), d.intent.clone());
        }
        MoteDef {
            critic_check: None,
            // Inherited from shaper (the heavy axes — D48 + D50).
            model_id: shaper.model_id.clone(),
            prompt_template_hash: shaper.prompt_template_hash,
            tool_contract: shaper.tool_contract.clone(),
            config_subset,
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
    use kx_mote::{ConfigVal, EffectPattern, LogicRef, NdClass, PromptTemplateHash, RoleId};
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
            intent: ConfigVal(Vec::new()),
        }
    }

    /// A descriptor carrying a non-empty per-child intent.
    fn descriptor_with_intent(
        seed: u8,
        nd: NdClass,
        ep: EffectPattern,
        intent: &[u8],
    ) -> ChildDescriptor {
        ChildDescriptor {
            intent: ConfigVal(intent.to_vec()),
            ..descriptor(seed, nd, ep)
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

    // --- T2: per-child intent behavior -----------------------------------

    #[test]
    fn empty_intent_inherits_shaper_config_verbatim() {
        // Behavior-preservation guarantee: an empty `intent` is a no-op, so the
        // child's `config_subset` is byte-identical to the shaper's — the same
        // resolved `MoteDef` (and `MoteId`) it had before the field existed.
        let shaper = shaper_def();
        let d = descriptor(7, NdClass::Pure, EffectPattern::IdempotentByConstruction);
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        assert_eq!(child.config_subset, shaper.config_subset);
        assert!(!child
            .config_subset
            .contains_key(&ConfigKey(PROMPT_KEY.to_string())));
    }

    #[test]
    fn nonempty_intent_overrides_only_the_prompt_key() {
        // A non-empty `intent` writes the prompt key and leaves every other
        // inherited config entry untouched (SN-8: it never touches an authority
        // axis — those are warrant/role-derived, not in config_subset here).
        let shaper = shaper_def(); // carries config["temperature"] = "0.0"
        let d = descriptor_with_intent(
            7,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
            b"summarize the corrected inputs",
        );
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        // Prompt key now holds the per-child intent...
        assert_eq!(
            child.config_subset.get(&ConfigKey(PROMPT_KEY.to_string())),
            Some(&ConfigVal(b"summarize the corrected inputs".to_vec())),
        );
        // ...and the shaper's other inherited config entries are preserved.
        assert_eq!(
            child.config_subset.get(&ConfigKey("temperature".into())),
            shaper.config_subset.get(&ConfigKey("temperature".into())),
        );
    }

    #[test]
    fn distinct_intent_changes_child_hash_same_intent_matches() {
        // Identity-bearing: two children differing ONLY by `intent` are
        // genuinely distinct work (different MoteDef hash). Same intent ⇒
        // byte-identical (determinism — the R49 replay property in miniature).
        let shaper = shaper_def();
        let a = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor_with_intent(
                1,
                NdClass::Pure,
                EffectPattern::IdempotentByConstruction,
                b"A",
            ),
        );
        let b = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor_with_intent(
                1,
                NdClass::Pure,
                EffectPattern::IdempotentByConstruction,
                b"B",
            ),
        );
        assert_ne!(a.hash(), b.hash(), "distinct intent ⇒ distinct child hash");

        let a2 = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor_with_intent(
                1,
                NdClass::Pure,
                EffectPattern::IdempotentByConstruction,
                b"A",
            ),
        );
        assert_eq!(a.hash(), a2.hash(), "same intent ⇒ identical child hash");
    }

    #[test]
    fn intent_overrides_an_inherited_prompt_when_the_shaper_has_one() {
        // When the shaper already carries a PROMPT_KEY (its planning prompt),
        // a non-empty child intent REPLACES it — the correction-fidelity fix:
        // the child runs its own instruction, not the shaper's planning prompt.
        let mut shaper = shaper_def();
        shaper.config_subset.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(b"PLAN the next steps".to_vec()),
        );
        let d = descriptor_with_intent(
            3,
            NdClass::Pure,
            EffectPattern::IdempotentByConstruction,
            b"DO step two",
        );
        let child = InheritFromShaperResolver.resolve(&shaper, &d);
        assert_eq!(
            child.config_subset.get(&ConfigKey(PROMPT_KEY.to_string())),
            Some(&ConfigVal(b"DO step two".to_vec())),
            "child intent must replace the shaper's planning prompt",
        );
        // And an EMPTY-intent child of the same shaper still inherits the
        // planning prompt (the override is gated on non-empty).
        let inheriting = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor(3, NdClass::Pure, EffectPattern::IdempotentByConstruction),
        );
        assert_eq!(
            inheriting
                .config_subset
                .get(&ConfigKey(PROMPT_KEY.to_string())),
            Some(&ConfigVal(b"PLAN the next steps".to_vec())),
        );
    }

    #[test]
    fn t6_intent_touches_only_the_prompt_key_never_an_authority_axis() {
        // SN-8: a per-child intent is model-proposed CONTENT — it must change
        // ONLY the prompt the child runs, never an authority/identity axis.
        // Two children differing solely by `intent` are identical on every
        // authority-relevant MoteDef field (tool_contract, nd_class,
        // effect_pattern, model_id) and on `config_subset` EXCEPT the prompt
        // key. (The warrant is not even a resolver output — it is computed by
        // the materializer via `intersect(shaper_warrant, role)`, which never
        // sees `intent` — so widening is structurally impossible here.)
        let shaper = shaper_def();
        let a = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor_with_intent(
                5,
                NdClass::Pure,
                EffectPattern::IdempotentByConstruction,
                b"X",
            ),
        );
        let b = InheritFromShaperResolver.resolve(
            &shaper,
            &descriptor_with_intent(
                5,
                NdClass::Pure,
                EffectPattern::IdempotentByConstruction,
                b"YY",
            ),
        );
        // Authority/identity axes besides the prompt are byte-identical.
        assert_eq!(a.tool_contract, b.tool_contract);
        assert_eq!(a.nd_class, b.nd_class);
        assert_eq!(a.effect_pattern, b.effect_pattern);
        assert_eq!(a.model_id, b.model_id);
        assert_eq!(a.inference_params, b.inference_params);
        assert_eq!(a.logic_ref, b.logic_ref);
        // config_subset differs ONLY at the prompt key.
        let prompt = ConfigKey(PROMPT_KEY.to_string());
        let strip = |m: &std::collections::BTreeMap<ConfigKey, ConfigVal>| {
            let mut m = m.clone();
            m.remove(&prompt);
            m
        };
        assert_eq!(
            strip(&a.config_subset),
            strip(&b.config_subset),
            "only the prompt key may differ between intent-varying children"
        );
        assert_ne!(
            a.config_subset.get(&prompt),
            b.config_subset.get(&prompt),
            "the prompt key DOES differ (the intent landed there)"
        );
    }
}
