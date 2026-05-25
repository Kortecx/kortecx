// Integration-test file: compiled as a separate crate from the host lib.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Concurrency tests for `kx-capability` (SN-4 v2 #6).
//!
//! - Compile-time `Send + Sync` over the full public-type set, including
//!   `Arc<dyn CapabilityBroker>` (proves the trait shape admits a hosted
//!   impl behind the same handle).
//! - 4-thread thread-independence of `dispatch` against a shared
//!   `Arc<LocalCapabilityBroker>` with deterministic capabilities —
//!   identical inputs across threads yield identical `BrokerHandle`s.
//! - 4-thread thread-independence of `idempotency_token_for`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use kx_capability::{
    idempotency_token_for, BrokerError, BrokerHandle, Capability, CapabilityBroker,
    CapabilityFailureReason, EffectRequest, LocalCapabilityBroker,
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
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Compile-time Send + Sync over the public surface (SN-4 v2 #6 part 1)
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    // Errors + reasons
    assert_send_sync::<BrokerError>();
    assert_send_sync::<CapabilityFailureReason>();

    // Request + handle
    assert_send_sync::<EffectRequest>();
    assert_send_sync::<BrokerHandle>();

    // Trait + impl
    assert_send_sync::<LocalCapabilityBroker<InMemoryContentStore>>();
    // The trait shape admits a hosted impl behind `Arc<dyn CapabilityBroker>`
    // — proves object-safety AND that the trait carries no in-process-only
    // generic parameter on the dyn surface.
    assert_send_sync::<Arc<dyn CapabilityBroker>>();
}

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// A deterministic capability that returns its payload XOR'd with a fixed key.
/// Used to assert thread-independence: identical inputs across threads
/// produce identical outputs, byte-for-byte.
struct XorCapability {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
}

impl Capability for XorCapability {
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
        Ok(request.payload.iter().map(|b| b ^ 0xA5).collect())
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
        schema_version: 3,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0u8; 32]),
        GraphPosition(b"/root".to_vec()),
        SmallVec::new(),
    )
}

fn request(payload: Vec<u8>) -> EffectRequest {
    EffectRequest {
        payload,
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence of dispatch (SN-4 v2 #6 part 2)
// ---------------------------------------------------------------------------

#[test]
fn dispatch_is_thread_independent() {
    let name = ToolName("xor".into());
    let version = ToolVersion("1".into());
    let mote = Arc::new(mote_with_tool(&name, &version));
    let warrant = Arc::new(permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    }));
    let broker = Arc::new(LocalCapabilityBroker::new(InMemoryContentStore::new()));
    broker.register_capability(Box::new(XorCapability {
        name: name.clone(),
        version,
        patterns: vec![EffectPattern::StageThenCommit],
    }));

    let name = Arc::new(name);
    let payload = Arc::new(vec![0x10, 0x20, 0x30, 0x40]);
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let b = Arc::clone(&broker);
            let m = Arc::clone(&mote);
            let w = Arc::clone(&warrant);
            let n = Arc::clone(&name);
            let p = Arc::clone(&payload);
            thread::spawn(move || {
                b.dispatch(&m, &w, &n, request((*p).clone()))
                    .expect("dispatch ok")
            })
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(
            first.staged_ref, r.staged_ref,
            "dispatch.staged_ref must be thread-independent (content-addressing yields identical refs for byte-identical responses)"
        );
        assert_eq!(
            first.capability, r.capability,
            "dispatch.capability must be thread-independent"
        );
        assert_eq!(
            first.capability_version, r.capability_version,
            "dispatch.capability_version must be thread-independent"
        );
    }
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence of idempotency_token_for
// ---------------------------------------------------------------------------

#[test]
fn idempotency_token_for_is_thread_independent() {
    let name = ToolName("xor".into());
    let version = ToolVersion("1".into());
    let mote = Arc::new(mote_with_tool(&name, &version));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let m = Arc::clone(&mote);
            thread::spawn(move || idempotency_token_for(&m))
        })
        .collect();
    let mut tokens = Vec::with_capacity(4);
    for h in handles {
        tokens.push(h.join().expect("worker did not panic"));
    }
    let first = tokens[0];
    for t in &tokens[1..] {
        assert_eq!(
            &first, t,
            "idempotency_token_for must be thread-independent (pure function of MoteId)"
        );
    }
}

// ---------------------------------------------------------------------------
// Register-then-dispatch under concurrent read/write — proves the RwLock
// correctly serializes write registrations against concurrent dispatches.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_register_and_dispatch_do_not_deadlock() {
    let name = ToolName("xor".into());
    let version = ToolVersion("1".into());
    let mote = Arc::new(mote_with_tool(&name, &version));
    let warrant = Arc::new(permissive_warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    }));
    let broker = Arc::new(LocalCapabilityBroker::new(InMemoryContentStore::new()));
    // Register the dispatch target up front.
    broker.register_capability(Box::new(XorCapability {
        name: name.clone(),
        version: version.clone(),
        patterns: vec![EffectPattern::StageThenCommit],
    }));

    // One thread keeps registering NEW capabilities under different names
    // (taking the write lock briefly); three threads keep dispatching the
    // already-registered one (taking the read lock).
    let name_for_dispatch = Arc::new(name);

    let writer_broker = Arc::clone(&broker);
    let writer = thread::spawn(move || {
        for i in 0u8..50 {
            let n = format!("aux-{i}");
            writer_broker.register_capability(Box::new(XorCapability {
                name: ToolName(n),
                version: ToolVersion("1".into()),
                patterns: vec![EffectPattern::StageThenCommit],
            }));
        }
    });

    let readers: Vec<_> = (0..3)
        .map(|_| {
            let b = Arc::clone(&broker);
            let m = Arc::clone(&mote);
            let w = Arc::clone(&warrant);
            let n = Arc::clone(&name_for_dispatch);
            thread::spawn(move || {
                for i in 0u8..50 {
                    let payload = vec![i, i.wrapping_add(1)];
                    let _ = b.dispatch(&m, &w, &n, request(payload));
                }
            })
        })
        .collect();

    writer.join().expect("writer did not panic");
    for r in readers {
        r.join().expect("reader did not panic");
    }
    // The dispatched capability is still resolvable.
    let h = broker
        .dispatch(&mote, &warrant, &name_for_dispatch, request(vec![1, 2, 3]))
        .expect("post-concurrency dispatch ok");
    // XOR with 0xA5: 1^0xA5, 2^0xA5, 3^0xA5
    let expected_bytes: Vec<u8> = vec![1u8 ^ 0xA5, 2u8 ^ 0xA5, 3u8 ^ 0xA5];
    assert_eq!(h.staged_ref, ContentRef::of(&expected_bytes));
}
