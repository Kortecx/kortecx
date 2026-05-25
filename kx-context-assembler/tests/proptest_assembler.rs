//! Property tests for `kx-context-assembler` (SN-4 v2 #6 — pinned per D33).
//!
//! Properties:
//!
//! 1. `assemble` is DETERMINISTIC — same inputs → byte-identical output.
//! 2. `assemble` is TOTAL — never panics on any input shape.
//! 3. Resolved bytes are NEVER hashes — the model never sees a `ContentRef`
//!    in `AssembledItem::bytes`.
//! 4. Parent order is STABLE — repeated assembly under the same shape produces
//!    items in the same order.
//! 5. `AssembledContext::content_ref` is DETERMINISTIC over the same bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_context_assembler::{assemble, AssembledContext, AssembledItem};
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
use proptest::prelude::*;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Test helpers
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

fn make_mote(mote_id: MoteId, parents: SmallVec<[ParentRef; 4]>) -> Mote {
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
            schema_version: kx_mote::MOTE_DEF_SCHEMA_VERSION,
        },
        input_data_id: InputDataId([0; 32]),
        graph_position: GraphPosition(vec![0]),
        parents,
    }
}

fn build_committed_entry(mote_id: MoteId, result_ref: ContentRef) -> JournalEntry {
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

/// Strategy: an arbitrary distinct seed byte (1..=200) used to mint MoteIds
/// and per-parent payload bytes.
fn arb_seed() -> impl Strategy<Value = u8> {
    1u8..=200
}

fn arb_payload() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=128)
}

// ---------------------------------------------------------------------------
// Build a deterministic registry/store/projection for property testing.
// ---------------------------------------------------------------------------

fn build_fixture(
    parents_data: &[(u8, Vec<u8>)],
) -> (
    InMemoryContentStore,
    kx_projection::Snapshot,
    InMemoryToolRegistry,
) {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    for (seed, payload) in parents_data {
        let r = store.put(payload).unwrap();
        let _ = journal
            .append(build_committed_entry(MoteId([*seed; 32]), r))
            .unwrap();
    }
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
    let registry = InMemoryToolRegistry::new();
    (store, snapshot, registry)
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 1: `assemble` is DETERMINISTIC.
    #[test]
    fn prop_assemble_is_deterministic(
        seed1 in arb_seed(),
        seed2 in arb_seed(),
        payload1 in arb_payload(),
        payload2 in arb_payload(),
    ) {
        prop_assume!(seed1 != seed2);
        let parents_data = vec![(seed1, payload1.clone()), (seed2, payload2.clone())];
        let (store, snapshot, registry) = build_fixture(&parents_data);

        let mote_parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
            ParentRef { parent_id: MoteId([seed1; 32]), edge: EdgeMeta::data() },
            ParentRef { parent_id: MoteId([seed2; 32]), edge: EdgeMeta::data() },
        ]);
        let mote = make_mote(MoteId([255; 32]), mote_parents);

        let a = assemble(&mote, &permissive_warrant(), &snapshot, &store, &registry, usize::MAX);
        let b = assemble(&mote, &permissive_warrant(), &snapshot, &store, &registry, usize::MAX);
        prop_assert_eq!(a, b);
    }

    /// Property 2: `assemble` is TOTAL (never panics on any input).
    #[test]
    fn prop_assemble_is_total(
        n_parents in 0usize..=4,
        seed in arb_seed(),
        payload in arb_payload(),
        window in 0usize..=10_000,
    ) {
        let parents_data: Vec<(u8, Vec<u8>)> = (0..n_parents)
            .map(|i| (seed.wrapping_add(i as u8 + 1), payload.clone()))
            .collect();
        let (store, snapshot, registry) = build_fixture(&parents_data);

        let mote_parents: SmallVec<[ParentRef; 4]> = parents_data.iter().map(|(s, _)| {
            ParentRef { parent_id: MoteId([*s; 32]), edge: EdgeMeta::data() }
        }).collect();
        let mote = make_mote(MoteId([254; 32]), mote_parents);

        // Reaching this assertion proves no panic.
        let _ = assemble(&mote, &permissive_warrant(), &snapshot, &store, &registry, window);
    }

    /// Property 3: resolved `bytes` are NEVER the ContentRef bytes (the model
    /// never sees a hash). I.e., `item.bytes != item.source_ref.0`.
    /// (True by construction unless the payload happens to equal a 32-byte
    /// hash — astronomically unlikely. We assert structurally: items emitted
    /// for parents always have `source_ref == ContentRef::of(bytes)`.)
    #[test]
    fn prop_resolved_bytes_match_source_ref_hashing(
        seed in arb_seed(),
        payload in arb_payload(),
    ) {
        let parents_data = vec![(seed, payload.clone())];
        let (store, snapshot, registry) = build_fixture(&parents_data);

        let mote = make_mote(
            MoteId([253; 32]),
            SmallVec::from_vec(vec![ParentRef {
                parent_id: MoteId([seed; 32]),
                edge: EdgeMeta::data(),
            }]),
        );

        let ctx = assemble(
            &mote, &permissive_warrant(), &snapshot, &store, &registry, usize::MAX,
        ).unwrap();

        prop_assert_eq!(ctx.items.len(), 1);
        let item = &ctx.items[0];
        // The bytes are the resolved payload, NOT the ref's raw bytes.
        prop_assert_eq!(&item.bytes[..], &payload[..]);
        // The source_ref hashes to those bytes.
        prop_assert_eq!(item.source_ref, ContentRef::of(&payload));
    }

    /// Property 4: parent order is STABLE — repeated calls under the same
    /// shape produce items in the same emission order.
    #[test]
    fn prop_parent_order_is_stable(
        s1 in arb_seed(),
        s2 in arb_seed(),
        s3 in arb_seed(),
        p1 in arb_payload(),
        p2 in arb_payload(),
        p3 in arb_payload(),
    ) {
        prop_assume!(s1 != s2 && s2 != s3 && s1 != s3);
        let parents_data = vec![
            (s1, p1.clone()), (s2, p2.clone()), (s3, p3.clone()),
        ];
        let (store, snapshot, registry) = build_fixture(&parents_data);

        let mote_parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
            ParentRef { parent_id: MoteId([s1; 32]), edge: EdgeMeta::data() },
            ParentRef { parent_id: MoteId([s2; 32]), edge: EdgeMeta::data() },
            ParentRef { parent_id: MoteId([s3; 32]), edge: EdgeMeta::data() },
        ]);
        let mote = make_mote(MoteId([252; 32]), mote_parents);

        let ctx = assemble(
            &mote, &permissive_warrant(), &snapshot, &store, &registry, usize::MAX,
        ).unwrap();

        // Verify sorted by source_ref's underlying MoteId — i.e., by seed byte.
        let mut sorted = [s1, s2, s3];
        sorted.sort();
        // Each item's source_ref should match the payload that the lowest-seed
        // parent had — i.e., the payload of `parents_data[seed→idx]` for the
        // smallest seed first.
        let payload_of = |seed: u8| {
            parents_data.iter().find(|(s, _)| *s == seed).map(|(_, p)| p.clone()).unwrap()
        };
        for (i, expected_seed) in sorted.iter().enumerate() {
            prop_assert_eq!(&ctx.items[i].bytes[..], &payload_of(*expected_seed)[..]);
        }
    }

    /// Property 5: `AssembledContext::content_ref` is DETERMINISTIC.
    #[test]
    fn prop_content_ref_is_deterministic(payloads in proptest::collection::vec(arb_payload(), 0..=5)) {
        let items: Vec<AssembledItem> = payloads.iter().map(|p| AssembledItem {
            label: "item".into(),
            bytes: Bytes::copy_from_slice(p),
            source_ref: ContentRef::of(p),
        }).collect();
        let ctx = AssembledContext { items };
        prop_assert_eq!(ctx.content_ref(), ctx.content_ref());
    }
}
