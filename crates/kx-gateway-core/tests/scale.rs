//! Scale-smoke: `GetProjection` folds the run's journal, which is inherently
//! O(journal length). This gate proves the fold + view mapping stays **linear**
//! (no accidental O(n^2)): the per-entry amortized time at 25k must stay within
//! 4x of the per-entry time at 1k. A quadratic regression would blow ~25x.
//!
//! `#[ignore]` — run via the `scale-smoke` recipe (`--release --ignored`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::sync::Arc;
use std::time::Instant;

use kx_content::{ContentRef, InMemoryContentStore};
use kx_gateway_core::{ContentReader, GatewayService, JournalReader, ReadOnly, RunSubmitter};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{MoteDefHash, MoteId, NdClass};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_server::KxGateway;
use tonic::Request;

const INSTANCE_ID: [u8; 16] = [0x11; 16];

struct NullSubmitter;
#[tonic::async_trait]
impl RunSubmitter for NullSubmitter {
    async fn register_run(
        &self,
        _r: [u8; 32],
    ) -> Result<[u8; 16], kx_gateway_core::SubmitterError> {
        Ok(INSTANCE_ID)
    }
    async fn submit_mote(
        &self,
        _m: kx_mote::Mote,
        _w: kx_warrant::WarrantSpec,
        _a: bool,
        _react_seed: bool,
    ) -> Result<kx_gateway_core::SubmitMoteOutcome, kx_gateway_core::SubmitterError> {
        unreachable!()
    }
}

fn mote_id(i: u32) -> MoteId {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&i.to_le_bytes());
    MoteId::from_bytes(b)
}

fn service_of_size(n: u32) -> GatewayService {
    let journal = InMemoryJournal::new();
    journal
        .append(JournalEntry::RunRegistered {
            instance_id: INSTANCE_ID,
            recipe_fingerprint: [0x22; 32],
            ts: 0,
            seq: 0,
        })
        .unwrap();
    for i in 0..n {
        journal
            .append(JournalEntry::Committed {
                mote_id: mote_id(i),
                idempotency_key: *mote_id(i).as_bytes(),
                seq: 0,
                nondeterminism: NdClass::Pure,
                result_ref: ContentRef::from_bytes(*mote_id(i).as_bytes()),
                parents: smallvec::SmallVec::new(),
                warrant_ref: ContentRef::from_bytes([0xaa; 32]),
                mote_def_hash: MoteDefHash::from_bytes(*mote_id(i).as_bytes()),
            })
            .unwrap();
    }
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(journal));
    let content: Arc<dyn ContentReader> = Arc::new(InMemoryContentStore::new());
    GatewayService::new(reader, content_submitter(), content)
}

fn content_submitter() -> Arc<dyn RunSubmitter> {
    Arc::new(NullSubmitter)
}

async fn time_get_projection(n: u32) -> f64 {
    let svc = service_of_size(n);
    let start = Instant::now();
    let view = svc
        .get_projection(Request::new(proto::GetProjectionRequest {
            instance_id: INSTANCE_ID.to_vec(),
            at_seq: None,
        }))
        .await
        .unwrap()
        .into_inner();
    let elapsed = start.elapsed().as_secs_f64();
    assert_eq!(view.motes.len(), n as usize, "all {n} motes rendered");
    elapsed / f64::from(n) // per-entry amortized
}

#[tokio::test]
#[ignore = "scale-smoke: run via `just scale-smoke` (--release --ignored)"]
async fn get_projection_fold_stays_linear() {
    let sizes = [1_000u32, 5_000, 10_000, 25_000];
    let mut per_entry = Vec::new();
    for n in sizes {
        let t = time_get_projection(n).await;
        per_entry.push(t);
        println!("n={n} per_entry={t:.3e}s");
    }
    let first = per_entry[0];
    let last = *per_entry.last().unwrap();
    assert!(
        last <= first * 4.0,
        "GetProjection fold went super-linear: per-entry {last:.3e}s @25k vs {first:.3e}s @1k (>4x)"
    );
}
