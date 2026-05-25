// Integration-test file: compiled as a separate crate from the host lib.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `kx-capability` (SN-4 v2 #5).
//!
//! Properties asserted (each 64 cases, the workspace default):
//!
//! 1. `prop_dispatch_is_deterministic` — for any byte payload, two
//!    sequential dispatches of the same `EffectRequest` against the same
//!    capability + warrant produce byte-identical `BrokerHandle`s
//!    (deterministic capability ⇒ deterministic staged_ref).
//! 2. `prop_dispatch_staged_ref_is_blake3_of_response` — the
//!    `staged_ref` returned by dispatch equals `ContentRef::of` over the
//!    exact bytes the capability returned. Pins the content-addressing
//!    contract (D17) at the broker boundary.
//! 3. `prop_idempotency_token_for_is_pure` — the helper is a pure
//!    function of `Mote.id`: two calls with the same Mote return the
//!    same 32 bytes; calls with different Motes return different bytes.
//! 4. `prop_unknown_capability_is_total` — for any capability name not
//!    in `mote.tool_contract`, dispatch returns `UnknownCapability` and
//!    never panics, never writes to the content store.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_capability::{
    idempotency_token_for, BrokerError, Capability, CapabilityBroker, CapabilityFailureReason,
    EffectRequest, LocalCapabilityBroker,
};
use kx_content::{ContentRef, InMemoryContentStore};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantSpec,
};
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// A deterministic capability: response is `payload + sentinel`.
struct AppendSentinelCapability {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
}

impl Capability for AppendSentinelCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn version(&self) -> &ToolVersion {
        &self.version
    }
    fn supported_patterns(&self) -> &[EffectPattern] {
        &self.patterns
    }
    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        let mut out = request.payload.clone();
        out.extend_from_slice(b"|done");
        Ok(out)
    }
}

fn permissive_warrant_with_grant(grant: ToolGrant) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::from([
                (PathBuf::from("/input"), FsMode::ReadOnly),
                (PathBuf::from("/output"), FsMode::ReadWrite),
            ]),
        },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::from([grant]),
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 8000,
            max_output_tokens: 2000,
            max_calls: 10,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 2000,
            mem_bytes: 4 << 30,
            wall_clock_ms: 60_000,
            fd_count: 256,
            disk_bytes: 4 << 30,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

fn mote_with_tool_and_position(name: &ToolName, version: &ToolVersion, pos: Vec<u8>) -> Mote {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(name.clone(), version.clone());
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([0u8; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0u8; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        schema_version: 3,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(pos),
        SmallVec::new(),
    )
}

/// Capability + supported_patterns containing the dispatch pattern.
fn standard_broker(
    name: &ToolName,
    version: &ToolVersion,
) -> LocalCapabilityBroker<InMemoryContentStore> {
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(AppendSentinelCapability {
        name: name.clone(),
        version: version.clone(),
        patterns: vec![EffectPattern::StageThenCommit],
    }));
    broker
}

fn request_with(payload: Vec<u8>) -> EffectRequest {
    EffectRequest {
        payload,
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// ---------------------------------------------------------------------------
// Property 1 — dispatch is deterministic
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_dispatch_is_deterministic(
        payload in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let name = ToolName("p1".into());
        let version = ToolVersion("1".into());
        let mote = mote_with_tool_and_position(&name, &version, b"/p1".to_vec());
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = standard_broker(&name, &version);

        let h1 = broker.dispatch(&mote, &warrant, &name, request_with(payload.clone())).unwrap();
        let h2 = broker.dispatch(&mote, &warrant, &name, request_with(payload)).unwrap();
        prop_assert_eq!(h1.staged_ref, h2.staged_ref);
        prop_assert_eq!(h1.capability, h2.capability);
        prop_assert_eq!(h1.capability_version, h2.capability_version);
    }
}

// ---------------------------------------------------------------------------
// Property 2 — staged_ref equals BLAKE3 of the bytes the capability returned
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_dispatch_staged_ref_is_blake3_of_response(
        payload in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let name = ToolName("p2".into());
        let version = ToolVersion("1".into());
        let mote = mote_with_tool_and_position(&name, &version, b"/p2".to_vec());
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        });
        let broker = standard_broker(&name, &version);

        let handle = broker.dispatch(&mote, &warrant, &name, request_with(payload.clone())).unwrap();
        // Known capability response: payload + "|done"
        let mut expected = payload;
        expected.extend_from_slice(b"|done");
        prop_assert_eq!(handle.staged_ref, ContentRef::of(&expected));
    }
}

// ---------------------------------------------------------------------------
// Property 3 — idempotency_token_for is pure (same input → same output;
// different positions → different MoteId → different output)
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_idempotency_token_for_is_pure(
        pos_a in proptest::collection::vec(any::<u8>(), 1..32),
        pos_b in proptest::collection::vec(any::<u8>(), 1..32),
    ) {
        prop_assume!(pos_a != pos_b);
        let name = ToolName("p3".into());
        let version = ToolVersion("1".into());
        let mote_a = mote_with_tool_and_position(&name, &version, pos_a.clone());
        let mote_a_again = mote_with_tool_and_position(&name, &version, pos_a);
        let mote_b = mote_with_tool_and_position(&name, &version, pos_b);

        let token_a = idempotency_token_for(&mote_a);
        let token_a_again = idempotency_token_for(&mote_a_again);
        let token_b = idempotency_token_for(&mote_b);

        // Same Mote (same graph_position) → same MoteId → same token
        prop_assert_eq!(&token_a, &token_a_again);
        prop_assert_eq!(&token_a, mote_a.id.as_bytes());
        // Different graph_position → different MoteId → different token
        prop_assert_ne!(&token_a, &token_b);
    }
}

// ---------------------------------------------------------------------------
// Property 4 — UnknownCapability is total + no content-store write
// ---------------------------------------------------------------------------

proptest! {
    #[test]
    fn prop_unknown_capability_is_total(
        random_name in "[a-z]{1,16}",
        payload in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let known = ToolName("p4-known".into());
        let known_ver = ToolVersion("1".into());
        let mote = mote_with_tool_and_position(&known, &known_ver, b"/p4".to_vec());
        let warrant = permissive_warrant_with_grant(ToolGrant {
            tool_id: known.clone(),
            tool_version: known_ver.clone(),
        });
        // Skip the case where the random name collides with the known one.
        prop_assume!(random_name != "p4-known");

        let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
        broker.register_capability(Box::new(AppendSentinelCapability {
            name: known,
            version: known_ver,
            patterns: vec![EffectPattern::StageThenCommit],
        }));
        // Also register the "random" name to prove the refusal comes from
        // the tool_contract check, not from the broker's registry miss.
        let probe = ToolName(random_name);
        broker.register_capability(Box::new(AppendSentinelCapability {
            name: probe.clone(),
            version: ToolVersion("1".into()),
            patterns: vec![EffectPattern::StageThenCommit],
        }));

        let result = broker.dispatch(&mote, &warrant, &probe, request_with(payload));
        match result {
            Err(BrokerError::UnknownCapability { name }) => prop_assert_eq!(name, probe),
            other => prop_assert!(false, "expected UnknownCapability, got {:?}", other),
        }
        // The no-write-on-failure property is asserted by the inline
        // `cap_5_capability_failure_no_content_store_write` test in
        // `src/lib.rs` (which has direct access to `broker.store`);
        // here the property is totality (no panic) under arbitrary
        // capability-name input.
    }
}
