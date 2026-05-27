//! Inline unit tests for kx-capability. Extracted per Rule 3 with bodies
//! unchanged.

use super::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, WarrantField, WarrantSpec,
};
use smallvec::SmallVec;

// -- Fixtures -----------------------------------------------------------

fn tool_name(name: &str) -> ToolName {
    ToolName(name.into())
}
fn tool_version(v: &str) -> ToolVersion {
    ToolVersion(v.into())
}

/// A capability that returns `payload.iter().rev().collect()` (the
/// reverse of the input bytes). Deterministic; useful for asserting
/// staged_ref values.
struct ReverseCapability {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
}

impl ReverseCapability {
    fn new(name: &str, version: &str, patterns: Vec<EffectPattern>) -> Self {
        Self {
            name: tool_name(name),
            version: tool_version(version),
            patterns,
        }
    }
}

impl Capability for ReverseCapability {
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
        Ok(request.payload.iter().rev().copied().collect())
    }
}

/// A capability that always fails with the given reason.
struct FailingCapability {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
    reason: CapabilityFailureReason,
}

impl Capability for FailingCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn version(&self) -> &ToolVersion {
        &self.version
    }
    fn supported_patterns(&self) -> &[EffectPattern] {
        &self.patterns
    }
    fn invoke(&self, _: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        Err(self.reason.clone())
    }
}

/// A capability whose probe returns Some(bytes) — for CAP-9 testing.
struct PrimedReadbackCapability {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
    prerecorded: Vec<u8>,
}

impl Capability for PrimedReadbackCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }
    fn version(&self) -> &ToolVersion {
        &self.version
    }
    fn supported_patterns(&self) -> &[EffectPattern] {
        &self.patterns
    }
    fn invoke(&self, _: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        // probe path should be taken before invoke; if invoke runs,
        // surface a distinct response so the test fails loudly.
        Ok(b"invoke-was-called-but-probe-should-have-fired".to_vec())
    }
    fn probe(&self, _: &EffectRequest) -> Result<Option<Vec<u8>>, CapabilityFailureReason> {
        Ok(Some(self.prerecorded.clone()))
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

/// Build a Mote whose `tool_contract` includes the given (name, version).
fn mote_with_tool(name: &ToolName, version: &ToolVersion) -> Mote {
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
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: 3,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(b"/root".to_vec()),
        SmallVec::new(),
    )
}

fn empty_request_with_pattern(pattern: EffectPattern, payload: Vec<u8>) -> EffectRequest {
    EffectRequest {
        payload,
        pattern,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// -- CAP-1 — dispatch returns content-addressed staged_ref -----------

#[test]
fn cap_1_dispatch_returns_content_addressed_handle() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "rev",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    let req = empty_request_with_pattern(EffectPattern::StageThenCommit, b"hello".to_vec());
    let handle = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect("dispatch ok");

    // The reverse of "hello" is "olleh" — the staged_ref is the hash of
    // those bytes.
    let expected = ContentRef::of(b"olleh");
    assert_eq!(handle.staged_ref, expected);
    assert_eq!(handle.capability, name);
    assert_eq!(handle.capability_version, version);
}

// -- CAP-2 — capability not in tool_contract → UnknownCapability -----

#[test]
fn cap_2_unknown_capability_when_not_in_tool_contract() {
    let known = tool_name("known");
    let known_ver = tool_version("1");
    let mote = mote_with_tool(&known, &known_ver);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: known.clone(),
        tool_version: known_ver.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    // Register the "known" capability but try to dispatch "other".
    broker.register_capability(Box::new(ReverseCapability::new(
        "known",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));
    broker.register_capability(Box::new(ReverseCapability::new(
        "other",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    let req = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![]);
    let other = tool_name("other");
    let err = broker
        .dispatch(&mote, &warrant, &other, req)
        .expect_err("dispatch should refuse");
    assert!(matches!(err, BrokerError::UnknownCapability { name } if name == other));
}

// -- CAP-3 — capability doesn't support pattern → UnsupportedPattern -

#[test]
fn cap_3_unsupported_pattern_when_capability_pattern_disjoint() {
    let name = tool_name("idem-only");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "idem-only",
        "1",
        vec![EffectPattern::IdempotentByConstruction],
    )));

    let req = empty_request_with_pattern(EffectPattern::ValidateThenCommit, vec![]);
    let err = broker
        .dispatch(&mote, &warrant, &name, req)
        .expect_err("dispatch should refuse");
    match err {
        BrokerError::UnsupportedPattern {
            capability,
            requested,
        } => {
            assert_eq!(capability, name);
            assert_eq!(requested, EffectPattern::ValidateThenCommit);
        }
        other => panic!("expected UnsupportedPattern, got {other:?}"),
    }
}

// -- CAP-4 — content-addressing dedupes identical responses ----------

#[test]
fn cap_4_identical_responses_dedupe_via_content_addressing() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "rev",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    let req1 = empty_request_with_pattern(EffectPattern::StageThenCommit, b"abc".to_vec());
    let req2 = empty_request_with_pattern(EffectPattern::StageThenCommit, b"abc".to_vec());
    let h1 = broker.dispatch(&mote, &warrant, &name, req1).unwrap();
    let h2 = broker.dispatch(&mote, &warrant, &name, req2).unwrap();
    // Distinct BrokerHandle structs but identical staged_ref because
    // the responses are byte-identical (ReverseCapability is
    // deterministic) — content-addressing dedupes.
    assert_eq!(h1.staged_ref, h2.staged_ref);
}

// -- CAP-5 — capability error produces CapabilityFailure -------------

#[test]
fn cap_5_capability_failure_no_content_store_write() {
    let name = tool_name("fail");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let store = InMemoryContentStore::new();
    let broker = LocalCapabilityBroker::new(store);
    broker.register_capability(Box::new(FailingCapability {
        name: name.clone(),
        version,
        patterns: vec![EffectPattern::StageThenCommit],
        reason: CapabilityFailureReason::RateLimited,
    }));

    let req = empty_request_with_pattern(EffectPattern::StageThenCommit, b"x".to_vec());
    let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
    assert!(matches!(
        err,
        BrokerError::CapabilityFailure {
            reason: CapabilityFailureReason::RateLimited,
            ..
        }
    ));
    // No write happened — `list_refs()` is empty.
    assert_eq!(
        broker.store.list_refs().count(),
        0,
        "no content-store write should occur on capability failure"
    );
}

// -- CAP-6 — capability not in warrant.tool_grants -------------------

#[test]
fn cap_6_capability_exceeds_warrant_on_tool_grants() {
    let name = tool_name("ungranted");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    // Warrant grants a DIFFERENT tool.
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: tool_name("other"),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "ungranted",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    let req = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![]);
    let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::ToolGrants
        }
    ));
}

// -- CAP-7a — request.net_scope ⊄ warrant.net_scope ------------------

#[test]
fn cap_7a_capability_exceeds_warrant_on_net_scope() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "rev",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    // Request needs egress to a host the warrant doesn't allow.
    let req = EffectRequest {
        payload: vec![],
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("evil.example.com:443".into())])),
        fs_scope: FsScope::empty(),
    };
    let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::NetScope
        }
    ));
}

// -- CAP-7b — request.fs_scope ⊄ warrant.fs_scope --------------------

#[test]
fn cap_7b_capability_exceeds_warrant_on_fs_scope() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "rev",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));

    // Request needs write to a path not in warrant's fs_scope.
    let req = EffectRequest {
        payload: vec![],
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/etc"), FsMode::ReadWrite)]),
        },
    };
    let err = broker.dispatch(&mote, &warrant, &name, req).unwrap_err();
    assert!(matches!(
        err,
        BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::FsScope
        }
    ));
}

// -- CAP-8 — idempotency_token_for returns mote.id bytes -------------

#[test]
fn cap_8_idempotency_token_for_returns_mote_id_bytes() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let token = idempotency_token_for(&mote);
    assert_eq!(token.len(), 32);
    assert_eq!(&token, mote.id.as_bytes());
}

// -- CAP-9 — probe_readback returns Some(handle) when capability has it

#[test]
fn cap_9_probe_readback_returns_some_when_capability_primes_a_readback() {
    let name = tool_name("primed");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    let prerecorded = b"already-applied-state".to_vec();
    broker.register_capability(Box::new(PrimedReadbackCapability {
        name: name.clone(),
        version,
        patterns: vec![EffectPattern::IdempotentByConstruction],
        prerecorded: prerecorded.clone(),
    }));

    let probe = empty_request_with_pattern(EffectPattern::IdempotentByConstruction, vec![]);
    let outcome = broker
        .probe_readback(&mote, &warrant, &name, probe)
        .expect("probe ok");
    let handle = outcome.expect("expected Some(handle) — capability primed the probe");
    assert_eq!(handle.staged_ref, ContentRef::of(&prerecorded));
}

// -- CAP-9b — default probe (no override) returns None ---------------

#[test]
fn cap_9b_default_probe_returns_none() {
    let name = tool_name("rev");
    let version = tool_version("1");
    let mote = mote_with_tool(&name, &version);
    let warrant = permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(ReverseCapability::new(
        "rev",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));
    let probe = empty_request_with_pattern(EffectPattern::StageThenCommit, vec![1, 2, 3]);
    let outcome = broker
        .probe_readback(&mote, &warrant, &name, probe)
        .expect("probe ok");
    assert!(
        outcome.is_none(),
        "default probe impl returns None — broker yields None"
    );
}

// -- Pattern: registered_count reflects registrations ---------------

#[test]
fn registered_count_reflects_register_calls() {
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    assert_eq!(broker.registered_count(), 0);
    broker.register_capability(Box::new(ReverseCapability::new(
        "a",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));
    assert_eq!(broker.registered_count(), 1);
    broker.register_capability(Box::new(ReverseCapability::new(
        "b",
        "1",
        vec![EffectPattern::StageThenCommit],
    )));
    assert_eq!(broker.registered_count(), 2);
    // Re-register same name → replaces, count unchanged.
    broker.register_capability(Box::new(ReverseCapability::new(
        "a",
        "2",
        vec![EffectPattern::StageThenCommit],
    )));
    assert_eq!(broker.registered_count(), 2);
}
