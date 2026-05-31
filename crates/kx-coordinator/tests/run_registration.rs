//! Run registration (M1.1, D63/D64): the coordinator assigns a fresh, journaled,
//! immutable `instance_id` at `RegisterRun` (the seq=1 `RunRegistered` fact) and
//! returns it. Identity is a *registered nonce*, not a content hash — re-running
//! the same recipe yields a NEW run with a NEW identity (the recipe fingerprint is
//! retained only for discovery/dedup). The nonce + clock are injected as seams so
//! these assertions are deterministic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;

use kx_coordinator::proto;
use kx_coordinator::proto::coordinator_server::Coordinator;
use kx_coordinator::{
    Clock, CoordinatorService, InMemoryWorkerRegistry, MoteState, RunNonceSource, WorkerRegistry,
};
use kx_journal::{InMemoryJournal, Journal, JournalEntry, SqliteJournal};
use tempfile::tempdir;
use tonic::{Code, Request};

/// A deterministic nonce source — every run gets the same injected `instance_id`
/// (production uses `OsRandomNonce`).
#[derive(Debug)]
struct FixedNonce([u8; 16]);

impl RunNonceSource for FixedNonce {
    fn fresh_instance_id(&self) -> [u8; 16] {
        self.0
    }
}

/// A fixed wall clock so the journaled `ts` is deterministic (it is audit-only;
/// never on the identity path).
#[derive(Debug)]
struct FixedClock(u64);

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.0
    }
}

fn registry() -> Arc<dyn WorkerRegistry> {
    Arc::new(InMemoryWorkerRegistry::new())
}

fn coordinator_with_nonce<J: Journal + Send + 'static>(
    journal: J,
    instance_id: [u8; 16],
    ts: u64,
) -> CoordinatorService {
    CoordinatorService::with_seams(
        journal,
        registry(),
        None,
        Arc::new(FixedClock(ts)),
        Arc::new(FixedNonce(instance_id)),
    )
}

/// Call the `RegisterRun` RPC and return the assigned 16-byte instance_id.
async fn register_run(svc: &CoordinatorService, recipe_fingerprint: [u8; 32]) -> [u8; 16] {
    let resp = svc
        .register_run(Request::new(proto::RegisterRunRequest {
            recipe_fingerprint: recipe_fingerprint.to_vec(),
        }))
        .await
        .unwrap()
        .into_inner();
    resp.instance_id
        .as_slice()
        .try_into()
        .expect("instance_id is 16 bytes")
}

#[tokio::test]
async fn register_run_appends_run_registered_at_seq_1() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let instance_id = [0xa1u8; 16];
    let fingerprint = [0xb2u8; 32];

    {
        let svc = coordinator_with_nonce(SqliteJournal::open(&path).unwrap(), instance_id, 42);
        let returned = register_run(&svc, fingerprint).await;
        assert_eq!(returned, instance_id, "RegisterRun returns the assigned id");
        assert_eq!(
            svc.run_registration().await.unwrap(),
            Some((instance_id, fingerprint)),
            "the projection surfaces the registered identity + fingerprint"
        );
    } // svc dropped → core thread exits → Sqlite handle closed

    // The seq=1 entry is the RunRegistered fact (read via an independent handle —
    // exercises encode → SQLite → decode round-trip).
    let journal = SqliteJournal::open(&path).unwrap();
    assert_eq!(
        journal.current_seq().unwrap(),
        1,
        "registration is the only fact"
    );
    let entries: Vec<JournalEntry> = journal.read_entries_by_seq(1..2).unwrap().collect();
    assert_eq!(entries.len(), 1);
    match &entries[0] {
        JournalEntry::RunRegistered {
            instance_id: got_id,
            recipe_fingerprint: got_fp,
            ts,
            seq,
        } => {
            assert_eq!(*got_id, instance_id);
            assert_eq!(*got_fp, fingerprint);
            assert_eq!(*ts, 42, "audit timestamp came from the injected clock");
            assert_eq!(*seq, 1, "RunRegistered is the FIRST entry of the run");
        }
        other => panic!("seq=1 entry must be RunRegistered, got {other:?}"),
    }
}

#[tokio::test]
async fn register_run_is_idempotent() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let instance_id = [0x5au8; 16];
    let fingerprint = [0x6bu8; 32];

    let svc = coordinator_with_nonce(SqliteJournal::open(&path).unwrap(), instance_id, 7);
    let first = register_run(&svc, fingerprint).await;
    // A second RegisterRun on the same run returns the SAME id, writes nothing new
    // (even if the client passes a different fingerprint — the fact is immutable).
    let second = register_run(&svc, [0xffu8; 32]).await;
    assert_eq!(first, second, "re-registration returns the existing id");
    drop(svc);

    // Exactly one RunRegistered entry exists (no second fact appended).
    let journal = SqliteJournal::open(&path).unwrap();
    let current = journal.current_seq().unwrap();
    let run_registered = journal
        .read_entries_by_seq(1..(current + 1))
        .unwrap()
        .filter(|e| matches!(e, JournalEntry::RunRegistered { .. }))
        .count();
    assert_eq!(run_registered, 1, "exactly one registration fact");
}

#[tokio::test]
async fn two_runs_same_recipe_get_distinct_instance_ids() {
    // The roadmap exit-gate assertion (D64): re-submitting an identical recipe (same
    // recipe_fingerprint) is a NEW run with a NEW, distinct registered identity.
    let fingerprint = [0x11u8; 32];
    let id_a = [0xaau8; 16];
    let id_b = [0xbbu8; 16];

    let svc_a = coordinator_with_nonce(InMemoryJournal::new(), id_a, 1);
    let svc_b = coordinator_with_nonce(InMemoryJournal::new(), id_b, 1);

    let run_a = register_run(&svc_a, fingerprint).await;
    let run_b = register_run(&svc_b, fingerprint).await;

    assert_ne!(
        run_a, run_b,
        "two runs of the same recipe → distinct identities"
    );
    assert_eq!(run_a, id_a);
    assert_eq!(run_b, id_b);
    // Same recipe → same fingerprint on both runs (discovery/dedup key).
    let (_, fp_a) = svc_a.run_registration().await.unwrap().unwrap();
    let (_, fp_b) = svc_b.run_registration().await.unwrap().unwrap();
    assert_eq!(fp_a, fingerprint);
    assert_eq!(fp_b, fingerprint);
}

#[tokio::test]
async fn recover_reads_instance_id_not_recomputed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let fingerprint = [0x33u8; 32];
    let original_id = [0xa0u8; 16];
    let different_id = [0xb0u8; 16];

    // Run A registers with `original_id`.
    {
        let svc_a = coordinator_with_nonce(SqliteJournal::open(&path).unwrap(), original_id, 1);
        assert_eq!(register_run(&svc_a, fingerprint).await, original_id);
    }

    // A fresh coordinator over the SAME journal — with a DIFFERENT nonce source —
    // recovers the registered identity from the log; the nonce is NEVER recomputed.
    let svc_b = coordinator_with_nonce(SqliteJournal::open(&path).unwrap(), different_id, 999);
    assert_eq!(
        svc_b.run_registration().await.unwrap(),
        Some((original_id, fingerprint)),
        "recovery reads the journaled instance_id, not the new nonce"
    );
    // Re-registering on the recovered run is idempotent — returns the ORIGINAL id,
    // not the fresh nonce, and appends nothing.
    assert_eq!(register_run(&svc_b, fingerprint).await, original_id);
    drop(svc_b);
    let journal = SqliteJournal::open(&path).unwrap();
    assert_eq!(
        journal.current_seq().unwrap(),
        1,
        "recovery + re-register wrote no second fact"
    );
}

#[tokio::test]
async fn submit_without_register_still_works() {
    // Back-compat (M1.1 is additive): a run that never calls RegisterRun submits +
    // commits exactly as before; run_registration() is simply None.
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    let mote = common::pure_root_mote();

    let worker = common::register(&svc, "w").await;
    common::submit(&svc, &mote, &warrant).await;
    common::commit(&svc, &mote, worker).await;

    assert_eq!(svc.committed_count().await.unwrap(), 1);
    assert_eq!(svc.state_of(mote.id).await.unwrap(), MoteState::Committed);
    assert_eq!(
        svc.run_registration().await.unwrap(),
        None,
        "no registration → no run identity (submit path unaffected)"
    );
}

#[tokio::test]
async fn register_run_refused_after_run_started() {
    // Registration must be the FIRST fact (seq=1). If a run begins without it,
    // RegisterRun is refused (FAILED_PRECONDITION) — the fact can never land mid-run.
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let warrant = common::sample_warrant();
    let mote = common::pure_root_mote();

    let worker = common::register(&svc, "w").await;
    common::submit(&svc, &mote, &warrant).await;
    common::commit(&svc, &mote, worker).await;

    let err = svc
        .register_run(Request::new(proto::RegisterRunRequest {
            recipe_fingerprint: vec![0x44u8; 32],
        }))
        .await
        .expect_err("registration after the run started must be refused");
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert_eq!(svc.run_registration().await.unwrap(), None);
}

#[tokio::test]
async fn register_run_rejects_bad_fingerprint_length() {
    let svc = CoordinatorService::new(InMemoryJournal::new());
    let err = svc
        .register_run(Request::new(proto::RegisterRunRequest {
            recipe_fingerprint: vec![0u8; 31], // not 32
        }))
        .await
        .expect_err("a non-32-byte fingerprint is rejected at the boundary");
    assert_eq!(err.code(), Code::InvalidArgument);
}
