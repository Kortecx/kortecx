//! Definition-of-Done for kx-proto (P2.1).
//!
//! Obligations:
//! 1. **Per-message round-trip** — every gRPC request/response prost-encodes and
//!    decodes back to an equal value.
//! 2. **Same-instance encode determinism** — encoding one value twice is stable
//!    (sanity check; protobuf bytes are NOT a canonical hash input — see #3).
//! 3. **Identity round-trip (keystone)** — domain -> proto -> domain preserves
//!    `MoteId` / `mote_def_hash` / `warrant_ref`. This is what proves the
//!    mirrored-fields schema honors the Rust-side identity invariant.
//! 4. **Boundary rejections** — UNSPECIFIED enums, wrong-length hashes, missing
//!    required messages, unknown enum values, and out-of-range scalars are typed
//!    errors, not silent coercions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use common::{sample_mote, sample_mote_def, sample_warrant};
use kx_proto::proto;
use kx_proto::ConvertError;
use kx_warrant::warrant_ref_of;
use prost::Message;

fn roundtrip<M>(m: &M)
where
    M: Message + Default + PartialEq + std::fmt::Debug,
{
    let bytes = m.encode_to_vec();
    let decoded = M::decode(&bytes[..]).expect("decode");
    assert_eq!(*m, decoded);
}

// --- Obligation 1: per-message round-trip ----------------------------------

#[test]
fn obligation_1_submit_mote_request_round_trips() {
    let req = proto::SubmitMoteRequest {
        mote: Some(sample_mote().into()),
        warrant: Some(sample_warrant().into()),
        accept_at_least_once: false,
        react_seed: false,
    };
    roundtrip(&req);
}

#[test]
fn obligation_1_submit_mote_response_round_trips() {
    roundtrip(&proto::SubmitMoteResponse {
        mote_id: vec![0xAB; 32],
        status: proto::SubmitStatus::Accepted as i32,
        detail: String::new(),
        instance_id: vec![0xCD; 16],
        refusal_code: "R-1".to_string(),
    });
}

#[test]
fn obligation_1_report_commit_request_round_trips() {
    let req = proto::ReportCommitRequest {
        mote_id: vec![1; 32],
        idempotency_key: vec![2; 32],
        result_ref: vec![3; 32],
        warrant_ref: vec![4; 32],
        mote_def_hash: vec![5; 32],
        nd_class: proto::NdClass::ReadOnlyNondet as i32,
        parents: vec![proto::ParentRef {
            parent_id: vec![6; 32],
            edge_kind: proto::EdgeKind::Data as i32,
            non_cascade: false,
        }],
        worker_id: 42,
    };
    roundtrip(&req);
}

#[test]
fn obligation_1_report_commit_response_round_trips() {
    roundtrip(&proto::ReportCommitResponse {
        committed_seq: 99,
        outcome: proto::CommitOutcome::Committed as i32,
        detail: "ok".into(),
    });
}

#[test]
fn obligation_1_heartbeat_round_trips() {
    roundtrip(&proto::HeartbeatRequest {
        worker_id: 7,
        timestamp_ms: 1_700_000_000_000,
        in_flight: 3,
    });
    roundtrip(&proto::HeartbeatResponse { ack: true });
}

#[test]
fn obligation_1_register_worker_round_trips() {
    roundtrip(&proto::RegisterWorkerRequest {
        executor_class: proto::ExecutorClass::Bwrap as i32,
        endpoint: "http://10.0.0.2:50051".into(),
    });
    roundtrip(&proto::RegisterWorkerResponse { worker_id: 7 });
}

#[test]
fn obligation_1_lease_work_request_round_trips() {
    roundtrip(&proto::LeaseWorkRequest {
        worker_id: 7,
        executor_class: proto::ExecutorClass::Bwrap as i32,
        max_motes: 16,
    });
}

#[test]
fn obligation_1_lease_work_response_round_trips() {
    let resp = proto::LeaseWorkResponse {
        items: vec![proto::WorkItem {
            mote: Some(sample_mote().into()),
            warrant: Some(sample_warrant().into()),
            parent_results: vec![proto::ParentResult {
                parent_mote_id: vec![0x11; 32],
                result_ref: vec![0x22; 32],
            }],
            tool_args: None,
        }],
        instance_id: vec![0xEF; 16],
    };
    roundtrip(&resp);
}

#[test]
fn obligation_1_lease_work_response_empty_round_trips() {
    roundtrip(&proto::LeaseWorkResponse {
        items: vec![],
        instance_id: vec![],
    });
}

#[test]
fn obligation_1_read_entries_request_round_trips() {
    roundtrip(&proto::ReadEntriesRequest {
        since_seq: 42,
        max: 256,
    });
}

#[test]
fn obligation_1_read_entries_response_round_trips() {
    let resp = proto::ReadEntriesResponse {
        entries: vec![proto::JournalEntry {
            seq: 7,
            kind: Some(proto::journal_entry::Kind::Committed(
                proto::CommittedEntry {
                    mote_id: vec![1; 32],
                    idempotency_key: vec![1; 32],
                    seq: 7,
                    nd_class: proto::NdClass::Pure as i32,
                    result_ref: vec![2; 32],
                    parents: vec![proto::ParentRef {
                        parent_id: vec![6; 32],
                        edge_kind: proto::EdgeKind::Data as i32,
                        non_cascade: false,
                    }],
                    warrant_ref: vec![4; 32],
                    mote_def_hash: vec![5; 32],
                },
            )),
        }],
        next_seq: 7,
    };
    roundtrip(&resp);
}

#[test]
fn obligation_1_read_entries_response_empty_round_trips() {
    roundtrip(&proto::ReadEntriesResponse {
        entries: vec![],
        next_seq: 0,
    });
}

// --- Obligation 2: same-instance encode determinism ------------------------

#[test]
fn obligation_2_encoding_same_value_is_stable() {
    let req = proto::SubmitMoteRequest {
        mote: Some(sample_mote().into()),
        warrant: Some(sample_warrant().into()),
        accept_at_least_once: false,
        react_seed: false,
    };
    assert_eq!(req.encode_to_vec(), req.encode_to_vec());
}

// --- Obligation 3: identity round-trip (the keystone) ----------------------

#[test]
fn obligation_3_mote_def_identity_round_trips() {
    let def = sample_mote_def();
    let wire: proto::MoteDef = def.clone().into();
    let back: kx_mote::MoteDef = wire.try_into().expect("convert");
    assert_eq!(def, back, "structural equality");
    assert_eq!(
        def.hash(),
        back.hash(),
        "mote_def_hash identity preserved across the wire mapping"
    );
}

#[test]
fn obligation_3_mote_id_identity_round_trips() {
    let mote = sample_mote();
    let wire: proto::Mote = mote.clone().into();
    let back: kx_mote::Mote = wire.try_into().expect("convert");
    assert_eq!(mote, back, "structural equality incl. re-derived MoteId");
    assert_eq!(mote.id, back.id, "MoteId identity preserved");
}

#[test]
fn obligation_3_warrant_ref_identity_round_trips() {
    let w = sample_warrant();
    let wire: proto::WarrantSpec = w.clone().into();
    let back: kx_warrant::WarrantSpec = wire.try_into().expect("convert");
    assert_eq!(w, back, "structural equality");
    assert_eq!(
        warrant_ref_of(&w),
        warrant_ref_of(&back),
        "warrant_ref identity preserved"
    );
}

#[test]
fn obligation_3_full_wire_pipeline_preserves_identity() {
    let mote = sample_mote();
    let warrant = sample_warrant();
    let req = proto::SubmitMoteRequest {
        mote: Some(mote.clone().into()),
        warrant: Some(warrant.clone().into()),
        accept_at_least_once: false,
        react_seed: false,
    };

    // Through the actual gRPC wire bytes and back.
    let bytes = req.encode_to_vec();
    let decoded = proto::SubmitMoteRequest::decode(&bytes[..]).expect("decode");

    let back_mote: kx_mote::Mote = decoded
        .mote
        .expect("mote present")
        .try_into()
        .expect("convert");
    let back_warrant: kx_warrant::WarrantSpec = decoded
        .warrant
        .expect("warrant present")
        .try_into()
        .expect("convert");

    assert_eq!(mote.id, back_mote.id, "MoteId survives the full pipeline");
    assert_eq!(
        warrant_ref_of(&warrant),
        warrant_ref_of(&back_warrant),
        "warrant_ref survives the full pipeline"
    );
}

#[test]
fn obligation_3_work_item_preserves_identity() {
    // The worker pulls a WorkItem and re-derives MoteId / warrant_ref from it to
    // build a ReportCommit the coordinator will accept; the wire mapping must
    // preserve both through encode + decode.
    let mote = sample_mote();
    let warrant = sample_warrant();
    let resp = proto::LeaseWorkResponse {
        items: vec![proto::WorkItem {
            mote: Some(mote.clone().into()),
            warrant: Some(warrant.clone().into()),
            parent_results: vec![],
            tool_args: None,
        }],
        instance_id: vec![0x01; 16],
    };

    let bytes = resp.encode_to_vec();
    let decoded = proto::LeaseWorkResponse::decode(&bytes[..]).expect("decode");
    let item = decoded.items.into_iter().next().expect("one item");

    let back_mote: kx_mote::Mote = item
        .mote
        .expect("mote present")
        .try_into()
        .expect("convert");
    let back_warrant: kx_warrant::WarrantSpec = item
        .warrant
        .expect("warrant present")
        .try_into()
        .expect("convert");

    assert_eq!(mote.id, back_mote.id, "MoteId survives the lease pipeline");
    assert_eq!(
        warrant_ref_of(&warrant),
        warrant_ref_of(&back_warrant),
        "warrant_ref survives the lease pipeline"
    );
}

#[test]
fn obligation_3_net_scope_none_round_trips() {
    let w = kx_warrant::WarrantSpec {
        net_scope: kx_warrant::NetScope::None,
        ..sample_warrant()
    };
    let wire: proto::WarrantSpec = w.clone().into();
    let back: kx_warrant::WarrantSpec = wire.try_into().expect("convert");
    assert_eq!(w, back);
    assert_eq!(warrant_ref_of(&w), warrant_ref_of(&back));
}

// --- Obligation 4: boundary rejections -------------------------------------

#[test]
fn obligation_4_unspecified_enum_rejected() {
    let mut wire: proto::MoteDef = sample_mote_def().into();
    wire.nd_class = proto::NdClass::Unspecified as i32;
    let err = kx_mote::MoteDef::try_from(wire).unwrap_err();
    assert_eq!(
        err,
        ConvertError::UnspecifiedEnum {
            enum_name: "NdClass"
        }
    );
}

#[test]
fn obligation_4_bad_hash_length_rejected() {
    let mut wire: proto::MoteDef = sample_mote_def().into();
    wire.logic_ref = vec![0u8; 31];
    let err = kx_mote::MoteDef::try_from(wire).unwrap_err();
    assert_eq!(
        err,
        ConvertError::BadHashLength {
            field: "MoteDef.logic_ref",
            len: 31
        }
    );
}

#[test]
fn obligation_4_missing_required_message_rejected() {
    let wire = proto::Mote {
        mote_id: vec![],
        def: None,
        input_data_id: vec![0; 32],
        graph_position: vec![],
        parents: vec![],
    };
    let err = kx_mote::Mote::try_from(wire).unwrap_err();
    assert_eq!(err, ConvertError::MissingField { field: "Mote.def" });
}

#[test]
fn obligation_4_unknown_enum_value_rejected() {
    let mut wire: proto::MoteDef = sample_mote_def().into();
    wire.nd_class = 99;
    let err = kx_mote::MoteDef::try_from(wire).unwrap_err();
    assert_eq!(
        err,
        ConvertError::UnknownEnum {
            enum_name: "NdClass",
            value: 99
        }
    );
}

#[test]
fn obligation_4_schema_version_out_of_range_rejected() {
    let mut wire: proto::MoteDef = sample_mote_def().into();
    wire.schema_version = u32::from(u16::MAX) + 1;
    let err = kx_mote::MoteDef::try_from(wire).unwrap_err();
    assert!(matches!(err, ConvertError::OutOfRange { .. }));
}
