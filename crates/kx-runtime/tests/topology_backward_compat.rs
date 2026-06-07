//! **T7 — backward-compat: the per-child-`intent` payload-format change is a
//! DELIBERATE, documented break.**
//!
//! When `ChildDescriptor` gained its `intent` field, the canonical-bincode
//! layout of a committed `TopologyDecision` changed (bincode is
//! non-self-describing, so the new field is positional, not optional). A
//! pre-intent committed payload (4-field descriptors) must NOT silently
//! mis-materialize under a post-intent binary — it must fail to decode. This
//! test pins that behavior so a future "lenient decode" cannot reintroduce a
//! silent identity divergence. (OSS regenerates `TopologyDecision`s in-memory
//! per run, so there is no shipped cross-version topology journal to migrate;
//! if that ever changes, a V1/V2 dual-format trial-decode is the migration to
//! add THEN — see `kx_runtime::decode_topology_decision` docs.)

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_mote::{
    canonical_config, ChildDescriptor, ConfigVal, EffectPattern, LogicRef, NdClass, RoleId,
    TopologyDecision,
};

/// Build canonical bytes for a single-child `TopologyDecision`, then strip the
/// trailing **empty `intent` length prefix** (an 8-byte fixint `0`, the last
/// field of the only child) to reproduce EXACTLY what an older 4-field binary
/// committed — without depending on a `serde`-derived legacy struct (which
/// would add a dev-dependency / perturb `Cargo.lock`).
fn legacy_four_field_bytes() -> Vec<u8> {
    let current = TopologyDecision {
        children: vec![ChildDescriptor {
            role_id: RoleId("worker".into()),
            logic_ref: LogicRef([7u8; 32]),
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            intent: ConfigVal(Vec::new()), // empty ⇒ 8-byte fixint length prefix = 0
        }],
    };
    let mut bytes = bincode::serde::encode_to_vec(&current, canonical_config())
        .expect("current TopologyDecision encodes");
    // Sanity: the canonical config uses fixed-int (u64) length encoding, so an
    // empty `ConfigVal` is exactly 8 trailing zero bytes.
    let tail = &bytes[bytes.len() - 8..];
    assert_eq!(
        tail, &[0u8; 8],
        "empty intent must encode as an 8-byte zero len"
    );
    bytes.truncate(bytes.len() - 8); // drop the intent field ⇒ a genuine 4-field stream
    bytes
}

#[test]
fn legacy_four_field_topology_payload_fails_to_decode() {
    let legacy_bytes = legacy_four_field_bytes();
    // The post-intent decoder expects a 5th field (`intent`'s length prefix)
    // after `effect_pattern`; the legacy stream ends there, so decode fails —
    // a clean, typed refusal, never a silent wrong-children materialization.
    let decoded = kx_runtime::decode_topology_decision(&legacy_bytes);
    assert!(
        decoded.is_err(),
        "a pre-intent (4-field) TopologyDecision payload must FAIL to decode \
         under the post-intent struct (deliberate, documented format break)"
    );
}

#[test]
fn current_payload_round_trips() {
    // Control: a current-format payload (built from the real types) decodes
    // cleanly — proving the legacy failure above is the field change, not a
    // broken fixture.
    let td = TopologyDecision {
        children: vec![ChildDescriptor {
            role_id: RoleId("worker".into()),
            logic_ref: LogicRef([7u8; 32]),
            nd_class: NdClass::Pure,
            effect_pattern: EffectPattern::IdempotentByConstruction,
            intent: ConfigVal(b"do the thing".to_vec()),
        }],
    };
    let bytes = bincode::serde::encode_to_vec(&td, canonical_config()).unwrap();
    let decoded = kx_runtime::decode_topology_decision(&bytes).expect("current payload decodes");
    assert_eq!(decoded.children.len(), 1);
    assert_eq!(
        decoded.children[0].intent,
        ConfigVal(b"do the thing".to_vec())
    );
}
