// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Concurrency tests for `kx-memoizer` (SN-4 v2 #7).
//!
//! - Compile-time `Send + Sync` over the full public-type set.
//! - 4-thread thread-independence of `lookup` (Arc<Snapshot> + Arc<Mote>;
//!   identical outcomes across threads — pins the "no thread-local state"
//!   contract that machine-independent replay requires).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_memoizer::{lookup, CacheHit};
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteDefHash,
    MoteId, NdClass, PromptTemplateHash,
};
use kx_projection::Projection;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Compile-time Send + Sync
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    assert_send_sync::<CacheHit>();
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence of `lookup`
// ---------------------------------------------------------------------------

fn make_pure_mote(mote_id: MoteId) -> Mote {
    Mote {
        id: mote_id,
        def: MoteDef {
            logic_ref: LogicRef([0; 32]),
            model_id: ModelId("m".into()),
            prompt_template_hash: PromptTemplateHash([0; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
        },
        input_data_id: InputDataId([0; 32]),
        graph_position: GraphPosition(vec![0]),
        parents: SmallVec::new(),
    }
}

fn build_committed(mote_id: MoteId, result_ref: ContentRef) -> JournalEntry {
    JournalEntry::Committed {
        mote_id,
        idempotency_key: mote_id.0,
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref,
        parents: SmallVec::new(),
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: MoteDefHash([0; 32]),
    }
}

#[test]
fn lookup_is_thread_independent_under_real_move() {
    let store = InMemoryContentStore::new();
    let rr = store.put(b"pure-payload").unwrap();
    let mid = MoteId([42; 32]);
    let journal = InMemoryJournal::new();
    let _ = journal.append(build_committed(mid, rr)).unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let mote = make_pure_mote(mid);

    let snapshot = Arc::new(snapshot);
    let mote = Arc::new(mote);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&snapshot);
            let m = Arc::clone(&mote);
            thread::spawn(move || lookup(&m, &s))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(first, r, "lookup must be thread-independent");
    }
    assert_eq!(*first, Some(CacheHit::Pure { result_ref: rr }));
}

#[test]
fn lookup_miss_is_thread_independent_under_real_move() {
    // Same shape as above, but with an empty projection — every thread
    // should observe a cache miss. Pins that misses are also
    // thread-independent (not just hits).
    let journal = InMemoryJournal::new();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
    let mote = make_pure_mote(MoteId([99; 32]));

    let snapshot = Arc::new(snapshot);
    let mote = Arc::new(mote);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&snapshot);
            let m = Arc::clone(&mote);
            thread::spawn(move || lookup(&m, &s))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(first, r, "lookup miss must be thread-independent");
    }
    assert_eq!(*first, None);
}
