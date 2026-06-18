//! The pure binding logic: resolve a published recipe by handle, validate the
//! supplied arguments, bind them into the recipe's variable config slots, compile
//! to runnable Motes, and narrow each Mote's warrant to the caller's authority.
//! No I/O, no async — fully unit-testable; this is the hard new D121 logic.

use std::collections::BTreeMap;

use kx_catalog::{
    free_params_to_input_schema, validate_args, AssetPath, AssetRef, BodyLedger, FreeParamContract,
    PartyId, SchemaResolver, SlotBinding, VersionLedger, VersionedContent,
};
use kx_mote::{encode_context_items, ConfigVal, ContextItemRef, Mote, MoteId, CONTEXT_ITEMS_KEY};
use kx_warrant::{intersect, Role, WarrantSpec};
use kx_workflow::compile;

use crate::error::InvokeError;

/// Resolve the caller's effective warrant for the `Use` action on an asset.
///
/// The implementation IS the authoritative ledger — a
/// [`GovernedCatalog`](kx_catalog::GovernedCatalog)
/// `resolve_effective_warrant_for(Use)` or a
/// `GovernedFleet::resolve_member_warrant(Use)`. kx-invoke resolves `Use`
/// authority through this seam and **never trusts a caller-supplied warrant**
/// (the no-privilege-escalation property, SN-8). `None` ⇒ not authorized.
pub trait UseWarrantResolver {
    /// The party's effective `Use` warrant on `asset`, or `None` if unauthorized.
    fn resolve_use(&self, party: &PartyId, asset: &AssetRef) -> Option<WarrantSpec>;
}

/// A recipe resolved + bound to concrete arguments, ready to submit.
#[derive(Debug, Clone)]
pub struct BoundRun {
    /// The recipe identity (its `ManifestId` bytes) — passed as the
    /// `recipe_fingerprint` to `RegisterRun` (discovery/dedup, never identity).
    pub recipe_fingerprint: [u8; 32],
    /// The runnable Motes in topological (submission) order, each paired with the
    /// warrant it runs under: `intersect(effective_use_warrant, step_warrant)` —
    /// ⊆ both, so a Mote can exceed neither the caller's grant nor the recipe's
    /// declaration (no-widen).
    pub motes: Vec<(Mote, WarrantSpec)>,
    /// The terminal (sink) Mote whose committed result is the invocation's output.
    pub terminal_mote_id: MoteId,
}

/// Resolve a published recipe by `handle`, bind `args_bytes`, and produce a
/// [`BoundRun`]. Authorization is enforced HERE (authoritative), not trusted from
/// the caller. The gate is **`Use`** via `use_resolver` (a team member's authority
/// is fleet-derived and may be `Use`-only, so `Use` — not `Read` — is the right
/// gate to invoke). It fires FIRST, before any resolve, so an unauthorized caller
/// learns nothing about what recipes exist (no existence oracle). Resolving the
/// handle → recipe identity is then an ungated lookup (already `Use`-authorized).
///
/// Then: fetch the executable body, validate args against the SAME `InputSchema`
/// the advertisement publishes (fail-closed), bind each variable slot into the
/// body's config (fail-closed if a slot binds no step), compile, and narrow each
/// Mote's warrant.
///
/// # Errors
/// See [`InvokeError`]. An untrusted inbound server must collapse the
/// authorization/existence variants to a uniform "not authorized" (no oracle).
#[allow(clippy::too_many_arguments)]
pub fn bind_snapshot<V, B, R, U>(
    versions: &V,
    bodies: &B,
    use_resolver: &U,
    party: &PartyId,
    handle: &AssetPath,
    free_params: &FreeParamContract,
    schema_resolver: &R,
    args_bytes: &[u8],
    context_items: &[ContextItemRef],
) -> Result<BoundRun, InvokeError>
where
    V: VersionLedger,
    B: BodyLedger,
    R: SchemaResolver,
    U: UseWarrantResolver,
{
    let asset = AssetRef::Path(handle.clone());

    // (1) Use authority — authoritative; never a caller-supplied warrant (SN-8).
    //     Fires FIRST so an unauthorized caller cannot probe recipe existence.
    let effective = use_resolver
        .resolve_use(party, &asset)
        .ok_or(InvokeError::Unauthorized)?;

    // (2) Resolve handle -> recipe identity (ungated lookup; Use already gated it).
    let manifest_id = match versions.resolve(handle) {
        Some((VersionedContent::Workflow(id), _)) => id,
        Some(_) => return Err(InvokeError::NotAWorkflow),
        None => return Err(InvokeError::NotFound),
    };

    // (3) The executable body for this recipe identity.
    let mut body = bodies
        .get_body(&manifest_id)
        .ok_or(InvokeError::BodyUnavailable)?;

    // (4) Validate args against the same schema the advertisement publishes.
    let schema = free_params_to_input_schema(free_params, schema_resolver)?;
    validate_args(&schema, args_bytes).map_err(|e| InvokeError::ArgValidation(format!("{e:?}")))?;

    // (5) Parse + bind each variable slot (fail-closed on a slot that binds no
    //     step — a silent drop could run the recipe with an unbound parameter).
    let arg_map: BTreeMap<String, serde_json::Value> = if args_bytes.is_empty() {
        BTreeMap::new()
    } else {
        serde_json::from_slice(args_bytes).map_err(|e| InvokeError::ArgParse(e.to_string()))?
    };
    for (name, slot) in &free_params.slots {
        if slot.binding != SlotBinding::Variable {
            continue; // Constant slots are fixed by the recipe body itself.
        }
        let value = arg_map
            .get(name)
            .ok_or_else(|| InvokeError::SlotMissing(name.clone()))?;
        // Canonical re-encoding of the validated value (no float reached here, so
        // the JSON bytes are deterministic: identical args -> identical identity).
        let bytes = serde_json::to_vec(value).map_err(|e| InvokeError::ArgParse(e.to_string()))?;
        if body.bind_param(name, &ConfigVal(bytes)) == 0 {
            return Err(InvokeError::SlotUnbound(name.clone()));
        }
    }

    // (5b) PR-7: inject the run's attached context-bundle items into every ENTRY
    //      step's identity-bearing `config_subset` (canonical-encoded). A different
    //      attached context ⇒ a different entry `MoteId` (exactly-once-per-
    //      `(input + context)`); an EMPTY attachment skips this entirely, so the
    //      bound motes are byte-identical to pre-PR-7 (canonical digest untouched).
    if !context_items.is_empty() {
        let encoded = ConfigVal(encode_context_items(context_items));
        body.inject_entry_config(CONTEXT_ITEMS_KEY, &encoded);
    }

    // (6) Compile + narrow each Mote's warrant to the caller's authority.
    let compiled = compile(&body).map_err(|e| InvokeError::Uncompilable(e.to_string()))?;
    let terminal_mote_id = compiled
        .motes
        .last()
        .map(|m| m.mote.id)
        .ok_or(InvokeError::EmptyRecipe)?;
    let mut motes = Vec::with_capacity(compiled.motes.len());
    for cm in &compiled.motes {
        // Narrow to least privilege: ⊆ the caller's effective Use authority AND
        // ⊆ the recipe's declared step warrant. `intersect` narrows a parent
        // WarrantSpec by a Role, so wrap the step warrant as the narrowing role.
        let step_role = Role {
            name: "recipe-step".to_string(),
            version: 0,
            spec: cm.warrant.clone(),
            description: String::new(),
        };
        let warrant = intersect(&effective, &step_role)?;
        motes.push((cm.mote.clone(), warrant));
    }

    Ok(BoundRun {
        recipe_fingerprint: manifest_id.0,
        motes,
        terminal_mote_id,
    })
}
