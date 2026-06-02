//! The vetted **role → recipe** seam. The model names a role; the runtime
//! supplies the heavy `MoteDef` axes from a [`RoleRecipe`] — never from model
//! output. This is the crate-private analog of the D48 `ChildResolver` /
//! `MoteDefRegistry` idea, kept inside `kx-planner` so it is NOT a change to
//! `kx-projection`'s traits (the thesis dependency-ban holds).
//!
//! Two registries, one key. A [`kx_mote::RoleId`] resolves against BOTH:
//! - the `kx_warrant::RoleRegistry` → a `Role` (the warrant template / capability),
//!   intersected with the parent warrant at lowering time (D75); and
//! - a [`RoleRecipeResolver`] → a [`RoleRecipe`] (the identity axes).
//!
//! Keying both on one `RoleId` keeps a step's warrant and `MoteDef` coherent.
//! (Forward note: M7's catalog unifies the two into one content-addressed role
//! catalog — see the SN-5 plan.)

use std::collections::BTreeMap;
use std::sync::RwLock;

use kx_critic_types::CheckSpec;
use kx_mote::{
    EffectPattern, InferenceParams, LogicRef, ModelId, NdClass, PromptTemplateHash, RoleId,
    ToolName, ToolVersion,
};

/// The vetted recipe for a role: every Mote-identity + capability axis the model
/// is **not** allowed to choose. Looked up by the same [`RoleId`] the warrant
/// `RoleRegistry` resolves, so a step's warrant and `MoteDef` flow from one named
/// role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRecipe {
    /// `MoteDef.logic_ref` — what code the step runs (vetted, never model output).
    pub logic_ref: LogicRef,
    /// `MoteDef.model_id` — the pinned model for the step.
    pub model_id: ModelId,
    /// `MoteDef.prompt_template_hash`.
    pub prompt_template_hash: PromptTemplateHash,
    /// `MoteDef.tool_contract` — the closed tool set, each pinned. MUST be a
    /// subset of the role's warrant `tool_grants` (enforced at lowering).
    pub tool_contract: BTreeMap<ToolName, ToolVersion>,
    /// The capability a WORLD-MUTATING / READ-ONLY-NONDET dispatch routes
    /// through (PURE steps ignore it), carried verbatim to submission. For a
    /// model step with no external tool this is the model pseudo-capability
    /// (e.g. `kx-model`); for a tool step it is the tool's name.
    pub capability: ToolName,
    /// `MoteDef.nd_class` — PURE / `ReadOnlyNondet` / `WorldMutating`.
    pub nd_class: NdClass,
    /// `MoteDef.effect_pattern`.
    pub effect_pattern: EffectPattern,
    /// `MoteDef.inference_params` (decoding params; identity-bearing, D50).
    pub inference_params: InferenceParams,
    /// The deterministic check — REQUIRED iff a step using this role is a
    /// deterministic critic; `None` otherwise.
    pub deterministic_check: Option<CheckSpec>,
}

/// Resolve a [`RoleId`] to its vetted [`RoleRecipe`].
///
/// MUST be deterministic over one plan lowering: resolving the same `RoleId`
/// twice MUST return identical recipes (replay-faithfulness rests on this, like
/// `kx_warrant::RoleRegistry`). Object-safe + `Send + Sync` so callers can hold
/// an `Arc<dyn RoleRecipeResolver>`.
pub trait RoleRecipeResolver: Send + Sync {
    /// Resolve a role's recipe, or `None` if the role is not registered.
    fn recipe(&self, role_id: &RoleId) -> Option<RoleRecipe>;
}

/// OSS-default [`RoleRecipeResolver`] backed by an in-memory `BTreeMap` (mirrors
/// `kx_warrant::InMemoryRoleRegistry`). Authors register recipes before lowering
/// a plan.
#[derive(Default, Debug)]
pub struct InMemoryRoleRecipes {
    recipes: RwLock<BTreeMap<RoleId, RoleRecipe>>,
}

impl InMemoryRoleRecipes {
    /// Construct an empty resolver.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a recipe under a role handle. Overwriting the same handle is
    /// permitted but is an author smell — determinism rests on registered
    /// recipes staying stable across a lowering.
    pub fn register(&self, role_id: RoleId, recipe: RoleRecipe) {
        if let Ok(mut map) = self.recipes.write() {
            map.insert(role_id, recipe);
        }
    }

    /// Number of registered recipes (useful for asserting setup in tests).
    pub fn len(&self) -> usize {
        self.recipes.read().map(|m| m.len()).unwrap_or(0)
    }

    /// `true` if no recipes are registered.
    pub fn is_empty(&self) -> bool {
        self.recipes.read().map(|m| m.is_empty()).unwrap_or(true)
    }
}

impl RoleRecipeResolver for InMemoryRoleRecipes {
    fn recipe(&self, role_id: &RoleId) -> Option<RoleRecipe> {
        self.recipes.read().ok()?.get(role_id).cloned()
    }
}
