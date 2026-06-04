// SPDX-License-Identifier: Apache-2.0
//! Mote-as-MCP advertisement (M7.3, D85) — **DESCRIPTOR ONLY**.
//!
//! Publishing a snapshot to the catalog's MCP surface makes it an MCP-callable
//! tool. This module produces the tool DESCRIPTOR `{name, description,
//! input_schema}` for a PUBLISHED, GRANTED snapshot. The inbound EXECUTION
//! endpoint (resolve → bind args → `RegisterRun` → `StageThenCommit` →
//! `Committed`) is **M8** (D121) — it is deliberately NOT built here, so the
//! catalog stays off the coordinator / journal and a half-built inbound path can
//! never leak authority.
//!
//! # The schema is the M8 contract
//!
//! The descriptor's [`InputSchema`] reuses the M5.3 closed, **no-float**
//! `kx_tool_registry::{InputSchema, ParamType}` — the SAME schema M8's inbound
//! path will hand to `kx_tool_registry::validate_args`, so validation is
//! forward-compatible by construction. A snapshot's free params are typed by
//! content-ref ([`crate::FreeParamSlot::schema_ref`]); the agreed payload format
//! is the **canonical bincode of a [`ParamType`]** ([`encode_param_schema`]),
//! resolved here via a caller-supplied [`SchemaResolver`] so kx-catalog needs no
//! content-store dependency.
//!
//! # Governance (D86)
//!
//! [`advertise_snapshot`] is gated on the LIVE [`crate::GrantLedger`]: only a
//! snapshot the querying party holds `Use` (or `Read`) on is advertised. A
//! revoked grant immediately stops advertisement; an ungranted party learns only
//! "unauthorized", never publication state (the gate is checked first).

use kx_tool_registry::{InputSchema, ParamSpec, ParamType};
use serde::{Deserialize, Serialize};

use crate::action::CatalogAction;
use crate::entry::{FreeParamContract, RecipeSnapshot, SlotBinding};
use crate::governed::GovernedCatalog;
use crate::ledger::GrantLedger;
use crate::party::PartyId;
use crate::path::{AssetPath, AssetRef};
use crate::signature::canonical_config;
use crate::version_ledger::VersionLedger;

/// Canonical-bincode bytes of a [`ParamType`] — the agreed
/// [`crate::FreeParamSlot::schema_ref`] payload format. M8's inbound path decodes
/// the SAME bytes for `validate_args`, so a snapshot published today advertises
/// and validates identically when execution lands. Float-free → infallible encode.
#[must_use]
pub fn encode_param_schema(ty: &ParamType) -> Vec<u8> {
    bincode::serde::encode_to_vec(ty, canonical_config())
        .expect("canonical bincode of a float-free ParamType is infallible")
}

/// Resolves a free-param slot's `schema_ref` → the canonical bincode bytes of a
/// [`ParamType`] (see [`encode_param_schema`]). Caller-supplied so kx-catalog
/// needs no content-store dependency — a `&dyn kx_dataset::DataStore` /
/// `kx_content::ContentStore` adapter is a one-line closure.
pub trait SchemaResolver {
    /// The canonical-bincode([`ParamType`]) bytes for `schema_ref`, or `None` if
    /// unknown.
    fn resolve_schema(&self, schema_ref: &[u8; 32]) -> Option<Vec<u8>>;
}

/// Any `Fn(&[u8; 32]) -> Option<Vec<u8>>` is a [`SchemaResolver`].
impl<F> SchemaResolver for F
where
    F: Fn(&[u8; 32]) -> Option<Vec<u8>>,
{
    fn resolve_schema(&self, schema_ref: &[u8; 32]) -> Option<Vec<u8>> {
        self(schema_ref)
    }
}

/// An MCP tool descriptor produced from a published, granted snapshot. DESCRIPTOR
/// ONLY (D85) — it carries NO execution endpoint; the inbound resolve→bind→
/// `RegisterRun`→`StageThenCommit` path is M8 (D121). This is exactly what M8's
/// server will expose (in `tools/list`) and validate proposed args against.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SnapshotAdvertisement {
    /// The MCP tool name (the snapshot's handle, `namespace/collection/name`).
    pub name: String,
    /// Human description (advisory; e.g. drawn from advisory metadata, D84).
    pub description: String,
    /// The typed input schema — the snapshot's `Variable` free-param slots, each
    /// `schema_ref` resolved to a [`ParamType`]. The SAME `InputSchema` M8 will
    /// pass to `validate_args`.
    pub input_schema: InputSchema,
    /// The exact asset this advertises (the exact-out anchor; not a fuzzy hit).
    pub asset: AssetRef,
}

/// Why [`advertise_snapshot`] refused. All fail-closed; no panics.
#[derive(Clone, PartialEq, Eq, Debug, thiserror::Error)]
pub enum AdvertiseError {
    /// The party holds neither `Use` nor `Read` on the asset (checked first, so no
    /// publication state leaks to an unauthorized party).
    #[error("party not authorized to {action:?} {asset}")]
    Unauthorized {
        /// The action whose absence blocked advertisement.
        action: CatalogAction,
        /// The asset it was required on (display form).
        asset: String,
    },
    /// No version is published at the handle.
    #[error("snapshot not published at handle {0}")]
    NotPublished(String),
    /// A `Variable` slot carries no `schema_ref`, so its MCP param cannot be typed.
    #[error("variable slot `{slot}` has no schema_ref (cannot type the MCP param)")]
    UntypedVariableSlot {
        /// The offending slot name.
        slot: String,
    },
    /// A `Variable` slot's `schema_ref` did not resolve to bytes.
    #[error("schema_ref for slot `{slot}` did not resolve")]
    SchemaUnresolved {
        /// The offending slot name.
        slot: String,
    },
    /// A slot's resolved bytes are not a valid canonical [`ParamType`] (fail-closed).
    #[error("schema bytes for slot `{slot}` are not a valid ParamType")]
    SchemaDecode {
        /// The offending slot name.
        slot: String,
    },
}

/// Produce an MCP tool descriptor from a published, granted snapshot.
///
/// Governance-gated (D86): the querying party MUST hold `Use` (or `Read`) on the
/// asset, checked against the LIVE [`GrantLedger`] via [`GovernedCatalog`] — an
/// ungranted snapshot is NEVER advertised. DESCRIPTOR ONLY (D85 / D121): it builds
/// `{name, description, input_schema}`; it does NOT execute and does NOT touch the
/// coordinator / journal.
///
/// # Errors
///
/// [`AdvertiseError::Unauthorized`] (gate fails), [`AdvertiseError::NotPublished`]
/// (handle resolves to nothing), or a `Schema*` error if a `Variable` slot's type
/// cannot be resolved/decoded.
pub fn advertise_snapshot<G: GrantLedger, V: VersionLedger>(
    governed: &GovernedCatalog<G, V>,
    party: &PartyId,
    handle: &AssetPath,
    snapshot: &RecipeSnapshot,
    description: &str,
    resolver: &impl SchemaResolver,
) -> Result<SnapshotAdvertisement, AdvertiseError> {
    let asset = AssetRef::Path(handle.clone());

    // (1) Governance gate FIRST, against the LIVE grant ledger — `Use` (the action
    // M8's inbound call will require) preferred, `Read` accepted (D86). Gate-first
    // ordering means an unauthorized party cannot probe publication state.
    let authorized = governed
        .grants()
        .is_authorized(party, &asset, CatalogAction::Use)
        || governed
            .grants()
            .is_authorized(party, &asset, CatalogAction::Read);
    if !authorized {
        return Err(AdvertiseError::Unauthorized {
            action: CatalogAction::Use,
            asset: asset.to_string(),
        });
    }

    // (2) Must be PUBLISHED (resolves on the version ledger).
    if governed.versions().resolve(handle).is_none() {
        return Err(AdvertiseError::NotPublished(handle.to_string()));
    }

    // (3) FreeParamContract → InputSchema (Variable slots only).
    let input_schema = free_params_to_input_schema(&snapshot.free_params, resolver)?;

    Ok(SnapshotAdvertisement {
        name: handle.to_string(),
        description: description.to_string(),
        input_schema,
        asset,
    })
}

/// Map a [`FreeParamContract`] to an [`InputSchema`]. Only `Variable` slots become
/// MCP params (a `Constant` slot is fixed by the recipe — not a free argument);
/// each `Variable` slot's `schema_ref` is resolved → canonical bincode → a
/// [`ParamType`]. Every `Variable` param is `required: true` and the schema is
/// `deny_unknown: true` (fail-closed against smuggled args, matching `validate_args`
/// strict mode). `BTreeMap` iteration gives a canonical param order.
///
/// Public so the M8 inbound-execution path (D121) binds args against the **same**
/// schema the advertisement publishes — forward-compatible by construction.
pub fn free_params_to_input_schema(
    contract: &FreeParamContract,
    resolver: &impl SchemaResolver,
) -> Result<InputSchema, AdvertiseError> {
    let mut params = Vec::new();
    for (name, slot) in &contract.slots {
        if slot.binding != SlotBinding::Variable {
            continue; // Constant slots are fixed by the recipe — not free args.
        }
        let schema_ref = slot
            .schema_ref
            .ok_or_else(|| AdvertiseError::UntypedVariableSlot { slot: name.clone() })?;
        let bytes = resolver
            .resolve_schema(&schema_ref)
            .ok_or_else(|| AdvertiseError::SchemaUnresolved { slot: name.clone() })?;
        let (ty, _) = bincode::serde::decode_from_slice::<ParamType, _>(&bytes, canonical_config())
            .map_err(|_| AdvertiseError::SchemaDecode { slot: name.clone() })?;
        params.push(ParamSpec {
            name: name.clone(),
            ty,
            required: true,
        });
    }
    Ok(InputSchema {
        params,
        deny_unknown: true,
    })
}
