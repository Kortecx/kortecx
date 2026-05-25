//! Concurrency tests for `kx-context-assembler` (SN-4 v2 #7).
//!
//! - Compile-time `Send + Sync` over the full public-type set.
//! - 4-thread thread-independence of `assemble` (Arc<>'d inputs; byte-identical
//!   `AssembledContext` across threads — pins the "no thread-local seed in
//!   BLAKE3/bincode" contract that machine-independent replay depends on).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_context_assembler::{assemble, AssembledContext, AssembledItem, AssemblyError};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    MoteDefHash, MoteId, NdClass, ParentRef, PromptTemplateHash,
};
use kx_projection::Projection;
use kx_tool_registry::InMemoryToolRegistry;
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    WarrantSpec,
};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Compile-time Send + Sync
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    assert_send_sync::<AssembledItem>();
    assert_send_sync::<AssembledContext>();
    assert_send_sync::<AssemblyError>();
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::from([(PathBuf::from("/input"), FsMode::ReadOnly)]),
        },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: BTreeSet::new(),
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

fn make_mote(parents: SmallVec<[ParentRef; 4]>) -> Mote {
    Mote {
        id: MoteId([99; 32]),
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
            schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
        },
        input_data_id: InputDataId([0; 32]),
        graph_position: GraphPosition(vec![0]),
        parents,
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
        mote_def_hash: MoteDefHash([0; 32]),
    }
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence
// ---------------------------------------------------------------------------

#[test]
fn assemble_is_thread_independent_under_real_move() {
    // Build a fixture with 3 parents.
    let store = InMemoryContentStore::new();
    let r_a = store.put(b"alpha bytes").unwrap();
    let r_b = store.put(b"beta bytes").unwrap();
    let r_c = store.put(b"gamma bytes").unwrap();
    let id_a = MoteId([1; 32]);
    let id_b = MoteId([2; 32]);
    let id_c = MoteId([3; 32]);

    let journal = InMemoryJournal::new();
    let _ = journal.append(build_committed(id_a, r_a)).unwrap();
    let _ = journal.append(build_committed(id_b, r_b)).unwrap();
    let _ = journal.append(build_committed(id_c, r_c)).unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let registry = InMemoryToolRegistry::new();
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
        ParentRef {
            parent_id: id_a,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: id_b,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: id_c,
            edge: EdgeMeta::data(),
        },
    ]);
    let mote = make_mote(parents);
    let warrant = permissive_warrant();

    // Share read-only across threads.
    let store = Arc::new(store);
    let snapshot = Arc::new(snapshot);
    let registry = Arc::new(registry);
    let mote = Arc::new(mote);
    let warrant = Arc::new(warrant);

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&store);
            let snap = Arc::clone(&snapshot);
            let reg = Arc::clone(&registry);
            let m = Arc::clone(&mote);
            let w = Arc::clone(&warrant);
            thread::spawn(move || assemble(&m, &w, &snap, &*s, &*reg, usize::MAX).expect("ok"))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }

    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(first, r, "assemble must be thread-independent");
    }
}

// ---------------------------------------------------------------------------
// content_ref is thread-independent
// ---------------------------------------------------------------------------

#[test]
fn content_ref_is_thread_independent() {
    use bytes::Bytes;
    let items = vec![
        AssembledItem {
            label: "a".into(),
            bytes: Bytes::from_static(b"alpha"),
            source_ref: ContentRef::from_bytes([1; 32]),
        },
        AssembledItem {
            label: "b".into(),
            bytes: Bytes::from_static(b"beta"),
            source_ref: ContentRef::from_bytes([2; 32]),
        },
    ];
    let ctx = Arc::new(AssembledContext { items });

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let c = Arc::clone(&ctx);
            thread::spawn(move || c.content_ref())
        })
        .collect();

    let mut refs = Vec::with_capacity(4);
    for h in handles {
        refs.push(h.join().expect("ok"));
    }
    let first = refs[0];
    for r in &refs[1..] {
        assert_eq!(&first, r, "content_ref must be thread-independent");
    }
}
