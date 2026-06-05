//! M3.1 — on-by-default capture (D67) integration.
//!
//! Proves the three load-bearing properties of the flip:
//! 1. **It works** — every committed Mote's action (its `result_ref`) is captured
//!    exactly once, `ActionsOnly` retains only the action join key.
//! 2. **It is byte-invisible to truth** — the product digest is identical whether
//!    capture is `None`, `ActionsOnly`, or inspected (capture is never journaled).
//! 3. **It survives recovery deterministically** — a replay re-derives a
//!    bit-identical ledger (capture is a pure function of the folded journal), and
//!    re-capture is idempotent (exactly-once per Mote).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use kx_runtime::config::Mode;
use kx_runtime::{engine, CaptureSink, RuntimeConfig};

fn cfg(dir: &std::path::Path, mode: Mode) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("journal.sqlite"),
        content_root: dir.join("content"),
        mode,
        crash_at: None,
        // Cadence 2 over the 8-Mote demo: capture co-exists with checkpointing
        // (both fire at the loop-bottom frontier) without perturbing either.
        checkpoint_every: Some(2),
        audit_log: None,
    }
}

#[test]
fn every_committed_mote_action_is_captured() {
    let dir = tempfile::tempdir().unwrap();
    let sink = CaptureSink::actions_only();
    let outcome = engine::run_with_capture(&cfg(dir.path(), Mode::Run), Some(&sink)).unwrap();
    assert!(outcome.is_complete(), "every workflow Mote commits");
    assert_eq!(outcome.committed, 8);

    let store = sink.store();
    // One captured action per committed Mote — exactly once (a single final sweep).
    assert_eq!(
        store.len(),
        outcome.committed,
        "exactly one captured action per committed Mote"
    );
    // ActionsOnly: each record holds the action join key and strips the rest.
    for (_id, rec) in store.iter() {
        assert!(
            rec.output_ref.is_some(),
            "the committed action result_ref is captured"
        );
        assert!(
            rec.input_ref.is_none(),
            "input is stripped under ActionsOnly"
        );
        assert!(
            rec.reasoning_ref.is_none(),
            "reasoning is stripped under ActionsOnly"
        );
        assert!(
            rec.thinking_ref.is_none(),
            "thinking is stripped under ActionsOnly"
        );
    }
    // Every declared canonical Mote is present (the 2 materialized children fill
    // out the count to 8).
    for id in engine::canonical_mote_ids() {
        assert!(store.get(&id).is_some(), "declared Mote {id:?} is captured");
    }
}

#[test]
fn capture_on_does_not_change_the_product_digest() {
    let on_dir = tempfile::tempdir().unwrap();
    let off_dir = tempfile::tempdir().unwrap();
    let sink = CaptureSink::actions_only();
    let on = engine::run_with_capture(&cfg(on_dir.path(), Mode::Run), Some(&sink)).unwrap();
    let off = engine::run_with_capture(&cfg(off_dir.path(), Mode::Run), None).unwrap();
    assert_eq!(
        on.digest, off.digest,
        "capture is OFF the truth path — the product digest is identical with it on or off"
    );
    assert_eq!(sink.len(), 8, "the `Some` run populated the ledger");
}

#[test]
fn capture_refs_are_deterministic_across_independent_runs() {
    // Captured action refs are content-addressed, so two independent clean runs
    // capture the identical `MoteId → result_ref` ledger — proving the captured
    // join key IS the real committed result, not an incidental value.
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let s1 = CaptureSink::actions_only();
    let s2 = CaptureSink::actions_only();
    engine::run_with_capture(&cfg(d1.path(), Mode::Run), Some(&s1)).unwrap();
    engine::run_with_capture(&cfg(d2.path(), Mode::Run), Some(&s2)).unwrap();
    let (a, b) = (s1.store(), s2.store());
    assert_eq!(a.len(), b.len());
    for (id, rec) in a.iter() {
        assert_eq!(
            b.get(id).unwrap().output_ref,
            rec.output_ref,
            "same Mote → same content-addressed action ref across runs"
        );
    }
}

#[test]
fn replay_recaptures_the_same_ledger_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let s_run = CaptureSink::actions_only();
    let run = engine::run_with_capture(&cfg(dir.path(), Mode::Run), Some(&s_run)).unwrap();
    assert_eq!(s_run.len(), 8);

    // A fresh process replaying the completed journal re-derives a bit-identical
    // ledger — capture is a pure function of the folded truth, and recording is
    // idempotent (overwrite by `MoteId`), so each action lands exactly once.
    let s_replay = CaptureSink::actions_only();
    let replay = engine::run_with_capture(&cfg(dir.path(), Mode::Replay), Some(&s_replay)).unwrap();
    assert_eq!(
        run.digest, replay.digest,
        "replay is bit-identical with capture on (capture cannot perturb recovery)"
    );
    assert_eq!(
        s_replay.len(),
        s_run.len(),
        "replay re-captures every committed action exactly once"
    );
    let (r, p) = (s_run.store(), s_replay.store());
    for (id, rec) in r.iter() {
        assert_eq!(
            p.get(id).unwrap().output_ref,
            rec.output_ref,
            "replay re-derives the identical action ref for {id:?}"
        );
    }
}
