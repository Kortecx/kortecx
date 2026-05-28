//! Restart/recovery: a coordinator built over a persistent Sqlite journal,
//! committed through, then dropped, and a fresh coordinator opened over the same
//! file recovers the committed state (the projection re-folds from the log) and
//! still dedupes a re-reported commit. This is the durable-recovery promise at the
//! distribution boundary — the journal, not the process, is the source of truth.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_coordinator::proto::CommitOutcome;
use kx_coordinator::{CoordinatorService, MoteState};
use kx_journal::SqliteJournal;
use tempfile::tempdir;

#[tokio::test]
async fn commit_survives_coordinator_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let warrant = common::sample_warrant();
    let mote = common::pure_root_mote();

    let committed_seq;
    {
        let svc = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
        let worker = common::register(&svc, "w").await;
        common::submit(&svc, &mote, &warrant).await;
        let commit = common::commit(&svc, &mote, worker).await;
        assert_eq!(commit.outcome, CommitOutcome::Committed as i32);
        committed_seq = commit.committed_seq;
        assert_eq!(svc.committed_count().await.unwrap(), 1);
    } // svc dropped → orchestration core exits → Sqlite connection closed

    // A fresh coordinator over the SAME journal file recovers the committed state.
    let svc2 = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
    assert_eq!(
        svc2.committed_count().await.unwrap(),
        1,
        "committed Mote recovered from the journal on restart"
    );
    assert_eq!(svc2.state_of(mote.id).await.unwrap(), MoteState::Committed);

    // Re-reporting after restart is a dedup-by-key hit (the recovered Mote counts
    // as admitted): same seq, no second write.
    let worker2 = common::register(&svc2, "w2").await;
    let again = common::commit(&svc2, &mote, worker2).await;
    assert_eq!(again.outcome, CommitOutcome::AlreadyCommitted as i32);
    assert_eq!(again.committed_seq, committed_seq);
    assert_eq!(svc2.committed_count().await.unwrap(), 1);
}

#[tokio::test]
async fn multi_mote_state_recovered_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("journal.db");
    let warrant = common::sample_warrant();

    // Commit two of three submitted Motes, then restart.
    let a = common::mote(1, kx_mote::NdClass::Pure, &[]);
    let b = common::mote(2, kx_mote::NdClass::Pure, &[]);
    let c = common::mote(3, kx_mote::NdClass::Pure, &[]);
    {
        let svc = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
        let worker = common::register(&svc, "w").await;
        for m in [&a, &b, &c] {
            common::submit(&svc, m, &warrant).await;
        }
        common::commit(&svc, &a, worker).await;
        common::commit(&svc, &b, worker).await;
        assert_eq!(svc.committed_count().await.unwrap(), 2);
    }

    let svc2 = CoordinatorService::new(SqliteJournal::open(&path).unwrap());
    assert_eq!(svc2.committed_count().await.unwrap(), 2);
    assert_eq!(svc2.state_of(a.id).await.unwrap(), MoteState::Committed);
    assert_eq!(svc2.state_of(b.id).await.unwrap(), MoteState::Committed);
    // C was never committed; after restart it has no journal entry, so the
    // recovered projection has no record of it (uncommitted work is re-derivable
    // from the workflow, not the log).
    assert_eq!(svc2.state_of(c.id).await.unwrap(), MoteState::Pending);
}
