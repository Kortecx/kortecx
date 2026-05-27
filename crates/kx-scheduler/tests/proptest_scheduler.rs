//! SN-4 v2 #5 — property tests covering the DAG-ordering invariant
//! across arbitrary linear chains and small diamond shapes.
//!
//! **The load-bearing property**: every dispatched Mote had all its
//! Data-edge parents in `Committed` state at the moment of dispatch.
//! That property is sourced from `Projection::ready_set()` — the
//! scheduler's job is only to honor it. The proptest is the
//! per-arbitrary-input executable form of "DAG ordering is correct."

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use kx_projection::{MoteState, Projection};
use kx_scheduler::{LocalPlacement, Scheduler};
use proptest::prelude::*;
use smallvec::{smallvec, SmallVec};

use crate::common::{
    committed_entry, data_parent, fold_or_panic, permissive_warrant, pure_mote, MockExecutor,
};

proptest! {
    /// Build a linear chain of N (2..=8) PURE Motes M1 → M2 → ... → MN.
    /// Submit all, then loop: tick, fold a Committed for each dispatched
    /// Mote, tick again. At steady state every Mote has been dispatched
    /// exactly once and every dispatched Mote's parents were Committed
    /// at the moment of dispatch.
    #[test]
    fn linear_chain_dispatches_in_parent_committed_order(
        n in 2usize..=8
    ) {
        let mut projection = Projection::new();
        let mut scheduler = Scheduler::new(LocalPlacement);
        let executor = MockExecutor::default();
        let warrant = permissive_warrant();

        // Build the chain.
        let mut motes = Vec::with_capacity(n);
        let mut prev: Option<kx_mote::Mote> = None;
        for i in 0..n {
            let pos = format!("/chain-{i}");
            let parents: SmallVec<[kx_mote::ParentRef; 4]> = match &prev {
                Some(p) => smallvec![data_parent(p)],
                None => SmallVec::new(),
            };
            let m = pure_mote(pos.as_bytes(), parents);
            motes.push(m.clone());
            prev = Some(m);
        }

        for m in &motes {
            scheduler.submit(kx_mote::Mote::clone(m), warrant.clone(), &mut projection).unwrap();
        }

        // Drain.
        let mut seq: u64 = 1;
        let mut total_dispatched = 0usize;
        let max_rounds = n * 2 + 4;
        let mut round = 0;
        while round < max_rounds {
            let s = scheduler.tick(&projection, &executor).unwrap();
            if s.dispatched.is_empty() {
                break;
            }
            for d in &s.dispatched {
                // Property: at dispatch, every Data-edge parent was Committed.
                let mote = motes.iter().find(|m| m.id == d.mote_id).unwrap();
                for p in &mote.parents {
                    prop_assert_eq!(
                        projection.state_of(&p.parent_id),
                        MoteState::Committed,
                        "parent of dispatched Mote must be Committed at dispatch time"
                    );
                }
                // Fold a Committed for this dispatched Mote so its children
                // can become ready.
                fold_or_panic(&mut projection, &committed_entry(mote, seq));
                seq += 1;
                total_dispatched += 1;
            }
            round += 1;
        }
        prop_assert_eq!(total_dispatched, n, "every Mote in the chain must be dispatched exactly once");
        prop_assert_eq!(scheduler.pending_count(), 0, "pending map must be empty after drain");
    }

    /// Build a small fan-out (one root, K children) and assert all K
    /// children dispatch on the tick AFTER the root commits.
    #[test]
    fn fanout_dispatches_all_children_after_root_commits(
        k in 1usize..=6
    ) {
        let mut projection = Projection::new();
        let mut scheduler = Scheduler::new(LocalPlacement);
        let executor = MockExecutor::default();
        let warrant = permissive_warrant();

        let root = pure_mote(b"/fan-root", SmallVec::new());
        scheduler.submit(root.clone(), warrant.clone(), &mut projection).unwrap();

        let mut children = Vec::with_capacity(k);
        for i in 0..k {
            let pos = format!("/fan-child-{i}");
            let c = pure_mote(pos.as_bytes(), smallvec![data_parent(&root)]);
            scheduler.submit(c.clone(), warrant.clone(), &mut projection).unwrap();
            children.push(c);
        }

        // Tick 1: only root.
        let s = scheduler.tick(&projection, &executor).unwrap();
        prop_assert_eq!(s.dispatched.len(), 1);
        prop_assert_eq!(s.dispatched[0].mote_id, root.id);

        fold_or_panic(&mut projection, &committed_entry(&root, 1));

        // Tick 2: all k children ready and dispatched.
        let s = scheduler.tick(&projection, &executor).unwrap();
        prop_assert_eq!(s.dispatched.len(), k, "all children must dispatch on the tick after root commits");
        let dispatched: std::collections::BTreeSet<_> =
            s.dispatched.iter().map(|d| d.mote_id).collect();
        let expected: std::collections::BTreeSet<_> = children.iter().map(|c| c.id).collect();
        prop_assert_eq!(dispatched, expected);
    }
}
