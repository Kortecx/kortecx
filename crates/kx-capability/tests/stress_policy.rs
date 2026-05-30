//! SN-8 security policy stress harness (P4.1 scale & performance validation campaign).
//!
//! `#[ignore]`d; run explicitly in RELEASE:
//!
//! ```text
//! cargo test -p kx-capability --release --test stress_policy \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Proves the SN-8 invariant under VOLUME — "the runtime ENFORCES (exact
//! crypto/subset equality); the model only proposes". Two campaigns:
//!
//! * **ALLOW volume** — 50_000 valid (`req ⊆ warrant`) requirement checks AND
//!   50_000 valid broker dispatches (`req` pattern + tool ⊆ warrant) — every one
//!   must be `Ok`.
//! * **DENY / adversarial volume** — 50_000 forged cases across the enforcement
//!   axes: capability-exceeds-warrant on each quantitative resource axis
//!   (cpu/mem/wall/fd/disk via `check_tool_requirement`), fs-widening and
//!   net-widening (`check_tool_requirement`), tool-grant WIDENING via
//!   `intersect` (`AttemptedWiden`), and unknown-capability via the broker
//!   (`UnknownCapability`). EVERY forged case must be REJECTED — zero admitted.
//!
//! The harness counts `allow_ok`, `deny_rejected`, and `any_forged_admitted`
//! (which MUST be 0).
//!
//! NOTE: the `LocalCapabilityBroker::precheck` path, the `kx-tool-registry`
//! lineage-subset (`InvalidLineageSubset`) path, and the `kx-model-validator`
//! (`ValidatorOutcome`) path are intentionally NOT exercised here — this file is
//! scoped to the kx-capability + kx-warrant enforcement surfaces it directly
//! depends on; the other crates have their own SN-8 proptests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use kx_capability::{
    BrokerError, Capability, CapabilityBroker, CapabilityFailureReason, EffectRequest,
    LocalCapabilityBroker,
};
use kx_content::{ContentRef, InMemoryContentStore};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::{
    check_tool_requirement, intersect, warrant_ref_of, ExecutorClass, FsMode, FsScope, Host,
    ModelRoute, MoteClass, NarrowingError, NetScope, ResourceCeiling, Role, ToolGrant,
    ToolRequirement, WarrantField, WarrantSpec,
};
use smallvec::SmallVec;

const ALLOW_N: usize = 50_000;
const DENY_N: usize = 50_000;

// --- fixtures ---------------------------------------------------------------

struct AppendCap {
    name: ToolName,
    version: ToolVersion,
    patterns: Vec<EffectPattern>,
}

impl Capability for AppendCap {
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

fn warrant_with_grant(grant: ToolGrant) -> WarrantSpec {
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

fn mote_with_tool(name: &ToolName, version: &ToolVersion, pos: Vec<u8>) -> Mote {
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
        GraphPosition(pos),
        SmallVec::new(),
    )
}

fn standard_broker(
    name: &ToolName,
    version: &ToolVersion,
) -> LocalCapabilityBroker<InMemoryContentStore> {
    let broker = LocalCapabilityBroker::new(InMemoryContentStore::new());
    broker.register_capability(Box::new(AppendCap {
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

fn zero_req() -> ToolRequirement {
    ToolRequirement {
        net_scope_required: NetScope::None,
        fs_scope_required: FsScope::empty(),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        min_resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
    }
}

// --- ALLOW volume -----------------------------------------------------------

fn run_allow() -> usize {
    let name = ToolName("allow-tool".into());
    let version = ToolVersion("1".into());
    let warrant = warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let mote = mote_with_tool(&name, &version, b"/allow".to_vec());
    let broker = standard_broker(&name, &version);

    let mut allow_ok = 0usize;

    // (a) 50k requirement checks that are within-warrant → all Ok.
    let ok_req = zero_req();
    for _ in 0..ALLOW_N {
        if check_tool_requirement(&ok_req, &warrant).is_ok() {
            allow_ok += 1;
        }
    }

    // (b) 50k broker dispatches with the granted tool + supported pattern → Ok.
    for i in 0..ALLOW_N {
        let payload = (i as u32).to_le_bytes().to_vec();
        if broker
            .dispatch(&mote, &warrant, &name, request_with(payload))
            .is_ok()
        {
            allow_ok += 1;
        }
    }

    assert_eq!(
        allow_ok,
        2 * ALLOW_N,
        "every within-warrant check + dispatch must be admitted"
    );
    allow_ok
}

// --- DENY / adversarial volume ----------------------------------------------

/// Returns (deny_rejected, forged_admitted).
fn run_deny() -> (usize, usize) {
    let name = ToolName("deny-tool".into());
    let version = ToolVersion("1".into());
    let warrant = warrant_with_grant(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    let mote = mote_with_tool(&name, &version, b"/deny".to_vec());
    let broker = standard_broker(&name, &version);

    let mut rejected = 0usize;
    let mut admitted = 0usize;

    // Distribute DENY_N cases across the forgery axes.
    // Axis set: 5 resource axes + fs widen + net widen + tool-grant widen +
    //           unknown capability = 9 axes.
    let axes = 9usize;
    let per_axis = DENY_N / axes;

    // 1..5: capability-exceeds-warrant on each quantitative resource axis.
    for axis in 0..5usize {
        for k in 0..per_axis {
            let mut rc = ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            };
            // Demand strictly MORE than the warrant grants on exactly one axis.
            let bump = (k as u64) + 1;
            match axis {
                0 => rc.cpu_milli = warrant.resource_ceiling.cpu_milli + (bump as u32),
                1 => rc.mem_bytes = warrant.resource_ceiling.mem_bytes + bump,
                2 => rc.wall_clock_ms = warrant.resource_ceiling.wall_clock_ms + bump,
                3 => rc.fd_count = warrant.resource_ceiling.fd_count + (bump as u32),
                _ => rc.disk_bytes = warrant.resource_ceiling.disk_bytes + bump,
            }
            let req = ToolRequirement {
                min_resource_ceiling: rc,
                ..zero_req()
            };
            if check_tool_requirement(&req, &warrant).is_err() {
                rejected += 1;
            } else {
                admitted += 1;
            }
        }
    }

    // 6: fs-scope WIDENING — require a mount the warrant does not grant.
    for _ in 0..per_axis {
        let req = ToolRequirement {
            fs_scope_required: FsScope {
                mounts: BTreeMap::from([(PathBuf::from("/etc/shadow"), FsMode::ReadWrite)]),
            },
            ..zero_req()
        };
        if check_tool_requirement(&req, &warrant).is_err() {
            rejected += 1;
        } else {
            admitted += 1;
        }
    }

    // 7: net-scope WIDENING — require egress to a host not on the allowlist.
    for _ in 0..per_axis {
        let req = ToolRequirement {
            net_scope_required: NetScope::EgressAllowlist(BTreeSet::from([Host(
                "evil.example.com:443".into(),
            )])),
            ..zero_req()
        };
        if check_tool_requirement(&req, &warrant).is_err() {
            rejected += 1;
        } else {
            admitted += 1;
        }
    }

    // 8: tool-grant WIDENING via intersect — a child role adds a grant not in
    //    the parent → AttemptedWiden { ToolGrants }.
    for k in 0..per_axis {
        let mut grants = warrant.tool_grants.clone();
        grants.insert(ToolGrant {
            tool_id: ToolName(format!("forged-{k}")),
            tool_version: ToolVersion("9".into()),
        });
        let mut child_spec = warrant.clone();
        child_spec.tool_grants = grants;
        let role = Role {
            name: "widen".into(),
            version: 1,
            spec: child_spec,
            description: String::new(),
        };
        match intersect(&warrant, &role) {
            Err(NarrowingError::AttemptedWiden {
                field: WarrantField::ToolGrants,
                ..
            }) => rejected += 1,
            Err(_) => rejected += 1, // any refusal still means NOT admitted
            Ok(_) => admitted += 1,
        }
    }

    // 9: unknown capability via the broker — dispatch a tool not in the Mote's
    //    tool_contract. Even though we REGISTER it on the broker, the contract
    //    check must refuse it (UnknownCapability).
    let probe = ToolName("unregistered-in-contract".into());
    broker.register_capability(Box::new(AppendCap {
        name: probe.clone(),
        version: ToolVersion("1".into()),
        patterns: vec![EffectPattern::StageThenCommit],
    }));
    let remainder = DENY_N - per_axis * axes;
    for _ in 0..(per_axis + remainder) {
        match broker.dispatch(&mote, &warrant, &probe, request_with(b"x".to_vec())) {
            Err(BrokerError::UnknownCapability { .. }) => rejected += 1,
            Err(_) => rejected += 1,
            Ok(_) => admitted += 1,
        }
    }

    (rejected, admitted)
}

#[test]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
fn sn8_policy_allow_and_deny_volume() {
    // warrant_ref_of determinism sanity (crypto equality basis).
    let w = warrant_with_grant(ToolGrant {
        tool_id: ToolName("x".into()),
        tool_version: ToolVersion("1".into()),
    });
    assert_eq!(warrant_ref_of(&w), warrant_ref_of(&w));

    let allow_start = Instant::now();
    let allow_ok = run_allow();
    let allow_ms = allow_start.elapsed().as_millis();

    let deny_start = Instant::now();
    let (deny_rejected, forged_admitted) = run_deny();
    let deny_ms = deny_start.elapsed().as_millis();

    assert_eq!(forged_admitted, 0, "SN-8: NO forged case may be admitted");

    println!(
        "POLICY: allow_ok={allow_ok} allow_ms={allow_ms} deny_rejected={deny_rejected} \
         deny_ms={deny_ms} forged_admitted={forged_admitted}"
    );
}
