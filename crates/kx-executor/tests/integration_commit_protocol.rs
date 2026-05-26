//! Structural integration tests on the PR 9b-2 commit_protocol scaffolding.
//! The trait body is unimplemented in this slice; tests verify the closed
//! error vocabulary (R-11 / R-12 / R-13 + 4 supporting variants) is
//! constructible, the `mote_id()` extractor is exhaustive, the
//! `is_recovery_refusal()` predicate identifies R-13 only, and the trait
//! type is object-safe + Send + Sync. Per-pattern impl + lifecycle
//! integration land in PR 9b-3+.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use kx_capability::EffectRequest;
use kx_content::ContentRef;
use kx_executor::{CommitInput, CommitProtocol, CommitProtocolError};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteDefHash,
    MoteId, NdClass, PromptTemplateHash, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;

fn sample_mote_id() -> MoteId {
    MoteId::from_bytes([0xAB; 32])
}

fn sample_content_ref() -> ContentRef {
    ContentRef::from_bytes([0xCD; 32])
}

fn pure_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
    }
}

fn build_idempotent_mote(seed: u8) -> Mote {
    let def = MoteDef {
        logic_ref: LogicRef::from_bytes([1; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([2; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(vec![seed]),
        SmallVec::new(),
    )
}

fn empty_effect_request() -> EffectRequest {
    EffectRequest {
        payload: Vec::new(),
        pattern: EffectPattern::IdempotentByConstruction,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
    }
}

// ============================================================================
// Variant constructibility — every variant in the closed vocabulary is
// reachable from caller code. The vocabulary is closed at PR 9b-2; future
// extensions land via new variants.
// ============================================================================

#[test]
fn r11_result_ref_incomplete_variant_constructs() {
    let err = CommitProtocolError::R11ResultRefIncomplete {
        mote_id: sample_mote_id(),
        result_ref: sample_content_ref(),
    };
    assert!(err.to_string().contains("R-11"));
    assert!(err.to_string().contains("missing or incomplete"));
}

#[test]
fn r12_committed_not_proof_of_validity_variant_constructs() {
    let err = CommitProtocolError::R12CommittedNotProofOfValidity {
        mote_id: sample_mote_id(),
        context: "audit-trail boundary at recovery fold".into(),
    };
    assert!(err.to_string().contains("R-12"));
    assert!(err.to_string().contains("NOT proof-of-validity"));
}

#[test]
fn r13_wm_redispatch_refused_variant_constructs() {
    let err = CommitProtocolError::R13WmReDispatchRefused {
        mote_id: sample_mote_id(),
        reason: "terminal_failure_observed".into(),
    };
    assert!(err.to_string().contains("R-13"));
    assert!(err.to_string().contains("terminal_failure_observed"));
}

#[test]
fn broker_dispatch_failed_variant_constructs() {
    let err = CommitProtocolError::BrokerDispatchFailed {
        mote_id: sample_mote_id(),
        reason: "remote unreachable".into(),
    };
    assert!(err.to_string().contains("broker dispatch failed"));
}

#[test]
fn content_store_put_failed_variant_constructs() {
    let err = CommitProtocolError::ContentStorePutFailed {
        mote_id: sample_mote_id(),
        reason: "disk full".into(),
    };
    assert!(err.to_string().contains("content store put failed"));
}

#[test]
fn journal_append_committed_failed_variant_constructs() {
    let err = CommitProtocolError::JournalAppendCommittedFailed {
        mote_id: sample_mote_id(),
        reason: "sqlite busy".into(),
    };
    assert!(err.to_string().contains("journal append(Committed) failed"));
}

#[test]
fn internal_variant_constructs() {
    let err = CommitProtocolError::Internal {
        mote_id: sample_mote_id(),
        reason: "unexpected".into(),
    };
    assert!(err.to_string().contains("internal error"));
}

// ============================================================================
// `mote_id()` extractor is total + correct on every variant. This is the
// canonical-classifier-cannot-drift test at the integration layer — if a
// new variant is added without a `mote_id()` match arm, this test won't
// compile.
// ============================================================================

#[test]
fn mote_id_extractor_returns_correct_id_for_every_variant() {
    let mid = sample_mote_id();
    let cases: Vec<CommitProtocolError> = vec![
        CommitProtocolError::R11ResultRefIncomplete {
            mote_id: mid,
            result_ref: sample_content_ref(),
        },
        CommitProtocolError::R12CommittedNotProofOfValidity {
            mote_id: mid,
            context: "ctx".into(),
        },
        CommitProtocolError::R13WmReDispatchRefused {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::BrokerDispatchFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::ContentStorePutFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::JournalAppendCommittedFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::Internal {
            mote_id: mid,
            reason: "r".into(),
        },
    ];
    for err in cases {
        assert_eq!(err.mote_id(), mid, "mote_id() must be total");
    }
}

// ============================================================================
// `is_recovery_refusal()` is true ONLY for R-13.
// ============================================================================

#[test]
fn is_recovery_refusal_identifies_r13_only() {
    let mid = sample_mote_id();
    assert!(CommitProtocolError::R13WmReDispatchRefused {
        mote_id: mid,
        reason: "any".into(),
    }
    .is_recovery_refusal());

    for err in [
        CommitProtocolError::R11ResultRefIncomplete {
            mote_id: mid,
            result_ref: sample_content_ref(),
        },
        CommitProtocolError::R12CommittedNotProofOfValidity {
            mote_id: mid,
            context: "ctx".into(),
        },
        CommitProtocolError::BrokerDispatchFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::ContentStorePutFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::JournalAppendCommittedFailed {
            mote_id: mid,
            reason: "r".into(),
        },
        CommitProtocolError::Internal {
            mote_id: mid,
            reason: "r".into(),
        },
    ] {
        assert!(
            !err.is_recovery_refusal(),
            "only R-13 is a recovery refusal; got: {err:?}",
        );
    }
}

// ============================================================================
// PartialEq + Clone on the closed vocabulary.
// ============================================================================

#[test]
fn commit_protocol_error_is_clone_and_eq() {
    let err = CommitProtocolError::R11ResultRefIncomplete {
        mote_id: sample_mote_id(),
        result_ref: sample_content_ref(),
    };
    let copy = err.clone();
    assert_eq!(err, copy);
}

// ============================================================================
// CommitInput structural test — non_exhaustive struct, with the required
// fields populated. PR 9b-3+ may add fields under #[non_exhaustive].
// ============================================================================

#[test]
fn commit_input_constructs_with_required_fields() {
    let mote = build_idempotent_mote(0xAB);
    let warrant = pure_warrant();
    let mote_def_hash = MoteDefHash::from_bytes([3; 32]);
    let input = CommitInput {
        mote: &mote,
        warrant: &warrant,
        capability: kx_mote::ToolName("publish".into()),
        effect_request: empty_effect_request(),
        warrant_ref: ContentRef::from_bytes([4; 32]),
        mote_def_hash,
        idempotency_key: [5; 32],
        parents: SmallVec::new(),
        diagnostic_context: "test",
    };
    assert_eq!(input.mote.id, mote.id);
    assert_eq!(input.diagnostic_context, "test");
}

// ============================================================================
// CommitProtocol trait is object-safe + Send + Sync. Future commit-protocol
// consumers hold `Arc<dyn CommitProtocol>`; this test pins the constraint at
// compile time.
// ============================================================================

struct StubCommitProtocol;

impl CommitProtocol for StubCommitProtocol {
    fn commit(&self, input: CommitInput<'_>) -> Result<u64, CommitProtocolError> {
        Err(CommitProtocolError::Internal {
            mote_id: input.mote.id,
            reason: "stub for object-safety test".into(),
        })
    }
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn commit_protocol_is_object_safe_send_sync() {
    let dyn_protocol: Arc<dyn CommitProtocol> = Arc::new(StubCommitProtocol);
    assert_send_sync::<Arc<dyn CommitProtocol>>();
    let mote = build_idempotent_mote(0xAB);
    let warrant = pure_warrant();
    let mid = mote.id;
    let input = CommitInput {
        mote: &mote,
        warrant: &warrant,
        capability: kx_mote::ToolName("publish".into()),
        effect_request: empty_effect_request(),
        warrant_ref: ContentRef::from_bytes([4; 32]),
        mote_def_hash: MoteDefHash::from_bytes([3; 32]),
        idempotency_key: [5; 32],
        parents: SmallVec::new(),
        diagnostic_context: "stub-test",
    };
    let r = dyn_protocol.commit(input);
    assert!(matches!(
        r,
        Err(CommitProtocolError::Internal { mote_id, .. }) if mote_id == mid
    ));
}
