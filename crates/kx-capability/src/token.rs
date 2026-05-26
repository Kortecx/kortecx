//! D38 §1 — derive the 32-byte idempotency token from a [`Mote`]'s identity.

use kx_mote::Mote;

/// Derive the 32-byte idempotency token from a [`Mote`]'s identity.
///
/// Per D38 §1, the broker passes this token through to a remote tool's
/// idempotency header (e.g., `Idempotency-Key: <hex>`) so that a recovery
/// re-dispatch of the same Mote produces the SAME token; the remote API
/// then returns the cached response and no double-effect occurs. The
/// 32-byte form is the raw [`MoteId`][kx_mote::MoteId] bytes; the caller
/// is free to hex-encode them per the remote API's wire format (e.g.,
/// `blake3::Hash::from_bytes(token).to_hex()`).
///
/// # Example
///
/// ```
/// use kx_capability::idempotency_token_for;
/// use kx_mote::{
///     EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
///     MoteDef, NdClass, PromptTemplateHash,
/// };
/// use smallvec::SmallVec;
/// use std::collections::BTreeMap;
///
/// let def = MoteDef {
///     logic_ref: LogicRef::from_bytes([0u8; 32]),
///     model_id: ModelId("m".into()),
///     prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
///     tool_contract: BTreeMap::new(),
///     nd_class: NdClass::Pure,
///     config_subset: BTreeMap::new(),
///     effect_pattern: EffectPattern::IdempotentByConstruction,
///     critic_for: None,
///     is_topology_shaper: false,
///     schema_version: 3,
/// };
/// let mote = Mote::new(
///     def,
///     InputDataId::from_bytes([0u8; 32]),
///     GraphPosition("/root".into()),
///     SmallVec::new(),
/// );
/// let token = idempotency_token_for(&mote);
/// assert_eq!(token.len(), 32);
/// assert_eq!(&token, mote.id.as_bytes());
/// ```
#[inline]
#[must_use]
pub fn idempotency_token_for(mote: &Mote) -> [u8; 32] {
    *mote.id.as_bytes()
}
