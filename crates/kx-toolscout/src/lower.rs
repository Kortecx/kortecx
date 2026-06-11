//! [`lower_to_workflow_def`] — a [`TaskBundle`] becomes a [`WorkflowDef`],
//! fail-closed, behind the exact-equality grant gate.
//!
//! Mirrors the two proven shapes: the per-tool step is the `react_tool_loop`
//! act step (`kx_workflow::generator` + a singleton `tool_contract` —
//! READ-ONLY-NONDET, `StageThenCommit`), and the refusal + seed discipline is
//! `kx-planner`'s (IMP-5 exact `(name, version)` membership BEFORE any step is
//! built; the seed derives from the committed bundle bytes, never a clock).
//! Scores are structurally absent here — the function signature admits none
//! (advisory-never-authorizes, SN-8).

use std::collections::BTreeMap;

use kx_bundle::TaskBundle;
use kx_mote::{ConfigKey, ConfigVal, EdgeMeta, LogicRef, ModelId, PROMPT_KEY};
use kx_warrant::{ToolGrant, WarrantSpec};
use kx_workflow::{compile, generator, CompiledWorkflow, WorkflowDef};

use crate::error::ToolScoutError;

/// The blake3 domain tag for the per-step sentinel [`LogicRef`]s — disjoint
/// from every gateway sentinel and every real logic hash by construction.
const STEP_LOGIC_DOMAIN: &[u8] = b"kx-toolscout/step/v1";

/// Derive step `i`'s sentinel logic ref: `blake3(domain ‖ fingerprint ‖ i)`.
/// Distinct per (bundle, position) so two steps running the same tool at
/// different positions keep distinct Mote identities.
fn step_logic(fingerprint: &[u8; 32], i: u32) -> LogicRef {
    let mut hasher = blake3::Hasher::new();
    hasher.update(STEP_LOGIC_DOMAIN);
    hasher.update(fingerprint);
    hasher.update(&i.to_le_bytes());
    LogicRef::from_bytes(*hasher.finalize().as_bytes())
}

/// Lower a bundle into a [`WorkflowDef`]: one generator step per sequenced
/// tool (singleton `tool_contract`, the bundle's intent as the identity-bearing
/// prompt config), chained by Data edges in sequence order.
///
/// **Why every step carries the FULL caller warrant** (a deliberate contrast
/// with `kx-planner`'s per-role `intersect(parent, role)` narrowing, D75):
/// this tier has no roles — a bundle is a flat tool chain under ONE intent,
/// so there is no per-step role template to intersect against. The warrant
/// passed in IS the executing authority the caller already holds: every
/// sequenced tool is pre-checked against it here (below), each step's
/// `tool_contract` is the SINGLETON for its own tool (so a step cannot
/// propose a sibling's tool), and the broker re-verifies the grant at
/// dispatch. Authority never widens; per-step narrowing arrives with the
/// role-bearing composer tier (kx-mcp-gateway, wave 3).
///
/// # Errors
///
/// - [`ToolScoutError::EmptyBundle`] — nothing to lower.
/// - [`ToolScoutError::UngrantedTool`] — a sequenced tool is not in
///   `warrant.tool_grants` by EXACT `(name, version)` equality. Checked for
///   the WHOLE sequence before any step is built (fail-closed; the
///   `kx-capability` precheck and `kx-toolcall` decode apply the same gate at
///   dispatch and decode — defense in depth, one semantics).
/// - [`ToolScoutError::Compile`] — an edge declaration failed (the fixed
///   chain shape never produces one; kept honest by propagation).
pub fn lower_to_workflow_def(
    bundle: &TaskBundle,
    warrant: &WarrantSpec,
    model_id: &ModelId,
    capability: &kx_mote::ToolName,
) -> Result<WorkflowDef, ToolScoutError> {
    if bundle.tool_sequence.is_empty() {
        return Err(ToolScoutError::EmptyBundle);
    }

    // The authority gate — before ANY step exists.
    for (name, version) in &bundle.tool_sequence {
        let grant = ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        };
        if !warrant.tool_grants.contains(&grant) {
            return Err(ToolScoutError::UngrantedTool {
                name: name.clone(),
                version: version.clone(),
            });
        }
    }

    let fingerprint = bundle.fingerprint();
    // Replay-stable seed from the bundle's content-addressed identity (the
    // planner's `seed_from_plan_bytes` shape) — never a clock.
    let seed = u32::from_le_bytes([
        fingerprint.0[0],
        fingerprint.0[1],
        fingerprint.0[2],
        fingerprint.0[3],
    ]);

    let mut wf = WorkflowDef::new(seed);
    let mut prev = None;
    for (i, (name, version)) in bundle.tool_sequence.iter().enumerate() {
        #[allow(clippy::cast_possible_truncation)]
        // SAFETY: a tool sequence longer than u32::MAX is unrepresentable in
        // practice (the bundle would not encode); positions stay exact.
        let mut step = generator(
            step_logic(&fingerprint.0, i as u32),
            model_id.clone(),
            warrant.clone(),
            capability.clone(),
        );
        step.tool_contract = BTreeMap::from([(name.clone(), version.clone())]);
        step.config_subset.insert(
            ConfigKey(PROMPT_KEY.to_string()),
            ConfigVal(bundle.intent.as_bytes().to_vec()),
        );
        let step_ref = wf.add_step(step);
        if let Some(parent) = prev {
            wf.add_edge(parent, step_ref, EdgeMeta::data())?;
        }
        prev = Some(step_ref);
    }
    Ok(wf)
}

/// [`lower_to_workflow_def`] then the **frozen** [`compile`] — the one-call
/// path from a bundle to a registered Mote DAG (the planner `compile_plan`
/// shape). `compile` derives every `MoteId`; nothing here hand-assigns
/// identity.
///
/// # Errors
///
/// Everything [`lower_to_workflow_def`] refuses, plus [`ToolScoutError::Compile`]
/// from the structural gate.
pub fn compile_bundle(
    bundle: &TaskBundle,
    warrant: &WarrantSpec,
    model_id: &ModelId,
    capability: &kx_mote::ToolName,
) -> Result<CompiledWorkflow, ToolScoutError> {
    let wf = lower_to_workflow_def(bundle, warrant, model_id, capability)?;
    Ok(compile(&wf)?)
}
