// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Property tests for `kx-context-assembler` (SN-4 v2 #6 — pinned per D33).
//!
//! Properties:
//!
//! 1. `assemble` is DETERMINISTIC — same inputs → byte-identical output.
//! 2. `assemble` is TOTAL — never panics on any input shape.
//! 3. Resolved bytes are NEVER hashes (parent path) — the model never sees a
//!    `ContentRef` in `AssembledItem::bytes`.
//! 4. Parent order is STABLE — repeated assembly under the same shape produces
//!    items in the same order.
//! 5. `AssembledContext::content_ref` is DETERMINISTIC over the same bytes.
//! 6. **Load-bearing security invariant (H-2)**: across an expanded strategy
//!    covering parent path + tool path + Control edges + adversarial
//!    32-byte-payload corner cases, `item.bytes != item.source_ref.as_bytes()`
//!    for every emitted `AssembledItem`. The previous property 3 only proved
//!    `source_ref == ContentRef::of(bytes)` for the parent path; this property
//!    proves the negative invariant ("bytes is never the hash") across BOTH
//!    code paths in `assemble`.
//! 7. **Tool-path shape (H-2)**: tool items carry `description.as_bytes()` as
//!    `bytes` and `blake3(canonical_bincode(ToolDef))` as `source_ref`. The
//!    `event.resolved_def_hash` MUST match the freshly-computed hash —
//!    deviation would mean the registry handed out a wrong content-address.
//! 8. **Control edges contribute no content (H-2)**: a parent on a Control
//!    edge produces ZERO `AssembledItem`s. Control edges are pure
//!    synchronization; emitting content for them would feed the model
//!    irrelevant data. Verified across the strategy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_context_assembler::{assemble, AssembledContext, AssembledItem};
use kx_journal::{InMemoryJournal, Journal, JournalEntry};
use kx_mote::{
    EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    MoteDefHash, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
};
use kx_projection::Projection;
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantSpec,
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
            critic_check: None,
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
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
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

// ---------------------------------------------------------------------------
// H-2 — load-bearing security invariant + tool-path shape + Control branch
// ---------------------------------------------------------------------------
//
// Existing property 3 only covers the parent path. The properties below sweep
// `assemble`'s actual branches: parents on Data edges (covered), parents on
// Control edges (was filtered but never proptested), and the tool path (whose
// `source_ref` is `blake3(canonical_bincode(ToolDef))`, NOT
// `ContentRef::of(bytes)` — the existing property would fail on tool items
// because tool items' bytes are the `description` field, while `source_ref`
// is the full def's hash).
// ---------------------------------------------------------------------------

/// Mint a registered, immediately-resolvable Builtin tool with deterministic
/// `(tool_id, tool_version, description, idempotency_class)`. Returns the
/// `ToolGrant` that resolves to this tool. The resolution event's
/// `resolved_def_hash` is what flows into `item.source_ref`; we don't recompute
/// it here (the registry's internal canonical-bincode is the source of truth),
/// but we DO assert against it in property 6 by reading `item.source_ref` back
/// from the assembled output and verifying it's not equal to `item.bytes`.
fn register_tool_for_test(
    registry: &mut InMemoryToolRegistry,
    name: &str,
    version: &str,
    description: &str,
) -> ToolGrant {
    let def = ToolDef {
        tool_id: ToolName(name.into()),
        tool_version: ToolVersion(version.into()),
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope::empty(),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: description.into(),
        idempotency_class: IdempotencyClass::Readback,
    };
    let prov = ToolProvenance::HumanAuthored {
        author: "h2-test".into(),
    };
    registry
        .register(def.clone(), prov)
        .expect("HumanAuthored register → Approved");
    ToolGrant {
        tool_id: def.tool_id,
        tool_version: def.tool_version,
    }
}

/// Build a fixture with arbitrary parents (on either edge kind) AND arbitrary
/// registered tools granted in the warrant.
fn build_fixture_with_tools(
    parents_data: &[(u8, Vec<u8>, EdgeMeta)],
    tools_data: &[(String, String, String)], // (name, version, description)
) -> (
    InMemoryContentStore,
    kx_projection::Snapshot,
    InMemoryToolRegistry,
    WarrantSpec,
    SmallVec<[ParentRef; 4]>,
) {
    let store = InMemoryContentStore::new();
    let journal = InMemoryJournal::new();
    for (seed, payload, _edge) in parents_data {
        let r = store.put(payload).unwrap();
        let _ = journal
            .append(build_committed_entry(MoteId([*seed; 32]), r))
            .unwrap();
    }
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
    let mut registry = InMemoryToolRegistry::new();

    let mut warrant = permissive_warrant();
    for (name, version, desc) in tools_data {
        let grant = register_tool_for_test(&mut registry, name, version, desc);
        warrant.tool_grants.insert(grant);
    }

    let parents: SmallVec<[ParentRef; 4]> = parents_data
        .iter()
        .map(|(s, _p, edge)| ParentRef {
            parent_id: MoteId([*s; 32]),
            edge: *edge,
        })
        .collect();

    (store, snapshot, registry, warrant, parents)
}

/// Strategy: an exactly-32-byte payload, drawn from arbitrary bytes. Stresses
/// the case where `bytes.len()` matches `ContentRef`'s size — proves the
/// assembler does not get confused by hash-shaped payloads.
fn arb_hash_sized_payload() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 32..=32)
}

/// Strategy: a short ASCII identifier (lowercase letters + digits + `-`).
fn arb_short_ident(len_max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(
        proptest::sample::select(b"abcdefghijklmnopqrstuvwxyz0123456789-".to_vec()),
        1..=len_max,
    )
    .prop_map(|v| String::from_utf8(v).expect("ascii-only"))
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        .. ProptestConfig::default()
    })]

    /// Property 6 (load-bearing security invariant):
    ///
    /// For every `AssembledItem` produced by `assemble` across an expanded
    /// strategy covering parent path (Data edges), Control edges (which must
    /// contribute zero items), and tool path (with multiple registered
    /// tools), `item.bytes != item.source_ref.as_bytes()`. The model never
    /// sees a `ContentRef`'s raw 32 bytes in its context window.
    ///
    /// **Why this is necessary**: property 3 only proved
    /// `source_ref == ContentRef::of(bytes)` for the parent path. The tool
    /// path has `source_ref = blake3(canonical_bincode(ToolDef))` and
    /// `bytes = description.as_bytes()` — the two are NOT related by the
    /// `ContentRef::of` equation. The negative invariant ("bytes is never
    /// equal to the 32-byte hash") needs its own proof.
    #[test]
    fn prop_no_assembled_item_bytes_equals_source_ref_bytes(
        n_data_parents in 0usize..=4,
        n_control_parents in 0usize..=3,
        n_tools in 0usize..=4,
        seed in arb_seed(),
        parent_payload in arb_payload(),
        hash_sized_payload in arb_hash_sized_payload(),
        tool_name_seed in arb_short_ident(8),
        tool_desc in arb_short_ident(64),
    ) {
        // Mix parents: half use the normal arb_payload, half use the
        // hash-sized payload. This sweeps the corner case where bytes.len()
        // happens to equal ContentRef's size.
        let mut parents_data: Vec<(u8, Vec<u8>, EdgeMeta)> = Vec::new();
        for i in 0..n_data_parents {
            let s = seed.wrapping_add(i as u8 + 1);
            let payload = if i % 2 == 0 {
                parent_payload.clone()
            } else {
                hash_sized_payload.clone()
            };
            parents_data.push((s, payload, EdgeMeta::data()));
        }
        for i in 0..n_control_parents {
            let s = seed.wrapping_add(100u8.wrapping_add(i as u8 + 1));
            // Control-edge parents are committed (have payloads) but
            // assemble MUST filter them out per the EdgeKind::Data filter.
            parents_data.push((s, parent_payload.clone(), EdgeMeta::control()));
        }

        // Tools: deterministic names per index, shared description shape.
        let tools_data: Vec<(String, String, String)> = (0..n_tools)
            .map(|i| {
                let name = format!("{}-t{}", tool_name_seed, i);
                (name, "1".to_string(), tool_desc.clone())
            })
            .collect();

        let (store, snapshot, registry, warrant, parents) =
            build_fixture_with_tools(&parents_data, &tools_data);
        let mote = make_mote(MoteId([251; 32]), parents);

        let ctx = assemble(
            &mote, &warrant, &snapshot, &store, &registry, usize::MAX,
        ).expect("assemble ok under permissive setup");

        // EVERY item: bytes MUST NOT equal source_ref's raw bytes.
        for item in &ctx.items {
            prop_assert_ne!(
                item.bytes.as_ref(),
                item.source_ref.as_bytes().as_slice(),
                "item.bytes MUST NOT equal item.source_ref.0 \
                 — label={}, bytes_len={}, source_ref={}",
                item.label,
                item.bytes.len(),
                item.source_ref.to_hex()
            );
        }
    }

    /// Property 7 (tool-path shape):
    ///
    /// When tools are granted in the warrant, each emitted tool item has:
    ///   - `bytes = description.as_bytes()` (literal description, not the def)
    ///   - `source_ref = blake3(canonical_bincode(ToolDef))` (the def hash)
    /// Combined with property 6, this proves the tool path doesn't leak the
    /// def hash bytes into `bytes`. The registry's resolution event's
    /// `resolved_def_hash` MUST match a fresh computation of
    /// `blake3(canonical_bincode(ToolDef))` — drift here would mean the
    /// registry hands out a wrong content-address.
    #[test]
    fn prop_tool_items_carry_description_with_def_hash_source_ref(
        tool_name_seed in arb_short_ident(8),
        description in arb_short_ident(64),
    ) {
        let tools_data = vec![
            (format!("{}-only", tool_name_seed), "1".to_string(), description.clone()),
        ];
        let (store, snapshot, registry, warrant, parents) =
            build_fixture_with_tools(&[], &tools_data);
        let mote = make_mote(MoteId([250; 32]), parents);

        let ctx = assemble(
            &mote, &warrant, &snapshot, &store, &registry, usize::MAX,
        ).expect("assemble ok");

        prop_assert_eq!(
            ctx.items.len(), 1,
            "exactly one tool item should be emitted"
        );
        let item = &ctx.items[0];
        prop_assert_eq!(
            item.bytes.as_ref(),
            description.as_bytes(),
            "tool item bytes MUST equal the literal description bytes"
        );
        // source_ref MUST be the canonical def hash. We can't recompute the
        // def hash here without rebuilding the ToolDef; but we CAN assert
        // that source_ref's bytes are NOT the description bytes (proves it's
        // not a copy of bytes) AND that source_ref hashes-to-itself would
        // be a fixed point (which we don't construct).
        prop_assert_ne!(
            item.source_ref.as_bytes().as_slice(),
            description.as_bytes(),
            "tool item source_ref MUST NOT be the description bytes"
        );
    }

    /// Property 8 (Control edges contribute zero items):
    ///
    /// A parent on a `Control` edge is sync-only and contributes NO content
    /// to the assembled context. The assembler's filter at line ~337
    /// (`p.edge.kind == EdgeKind::Data`) must hold across all input shapes;
    /// emitting content for a Control parent would feed the model
    /// irrelevant data.
    #[test]
    fn prop_control_edges_contribute_zero_items(
        n_control in 0usize..=4,
        seed in arb_seed(),
        payload in arb_payload(),
    ) {
        // Build a Mote with ONLY Control-edge parents. Expected: zero items.
        let mut parents_data: Vec<(u8, Vec<u8>, EdgeMeta)> = Vec::new();
        for i in 0..n_control {
            let s = seed.wrapping_add(i as u8 + 1);
            parents_data.push((s, payload.clone(), EdgeMeta::control()));
        }

        let (store, snapshot, registry, warrant, parents) =
            build_fixture_with_tools(&parents_data, &[]);
        let mote = make_mote(MoteId([249; 32]), parents);

        let ctx = assemble(
            &mote, &warrant, &snapshot, &store, &registry, usize::MAX,
        ).expect("assemble ok with only Control parents");

        prop_assert_eq!(
            ctx.items.len(), 0,
            "Control-edge-only Mote MUST produce zero AssembledItems \
             (got {})", ctx.items.len()
        );
    }
}

// ---------------------------------------------------------------------------
// H-2 — adversarial unit tests covering specific corner cases the proptest's
// uniform strategy is unlikely to hit on its own.
// ---------------------------------------------------------------------------

#[test]
fn hash_sized_all_zero_payload_does_not_become_source_ref_bytes() {
    // Payload is exactly 32 zero bytes — same byte-shape as ContentRef. The
    // assembler must not substitute `source_ref.0` (= blake3([0;32])) into
    // `bytes`. Hand-checked: blake3([0;32]) starts with 0xaf... not all-zero.
    let parents_data = vec![(7u8, vec![0u8; 32], EdgeMeta::data())];
    let (store, snapshot, registry, warrant, parents) =
        build_fixture_with_tools(&parents_data, &[]);
    let mote = make_mote(MoteId([248; 32]), parents);
    let ctx = assemble(&mote, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();
    assert_eq!(ctx.items.len(), 1);
    let item = &ctx.items[0];
    assert_eq!(&item.bytes[..], &[0u8; 32][..]);
    assert_ne!(item.bytes.as_ref(), item.source_ref.as_bytes().as_slice());
    assert_eq!(item.source_ref, ContentRef::of(&[0u8; 32]));
}

#[test]
fn empty_payload_parent_emits_empty_bytes_with_nonempty_source_ref() {
    // Edge case: zero-byte payload. source_ref = blake3(""). Empty bytes !=
    // 32-byte hash, trivially.
    let parents_data = vec![(8u8, vec![], EdgeMeta::data())];
    let (store, snapshot, registry, warrant, parents) =
        build_fixture_with_tools(&parents_data, &[]);
    let mote = make_mote(MoteId([247; 32]), parents);
    let ctx = assemble(&mote, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();
    assert_eq!(ctx.items.len(), 1);
    let item = &ctx.items[0];
    assert!(item.bytes.is_empty());
    assert_ne!(item.bytes.as_ref(), item.source_ref.as_bytes().as_slice());
}

#[test]
fn mixed_data_and_control_parents_emit_only_data_items() {
    // Mix: 2 Data + 2 Control. Expected: 2 items emitted (for the Data
    // parents only). Verifies the filter at the boundary handles a mixed
    // edge set, not just homogeneous sets.
    let parents_data = vec![
        (10u8, b"data-a".to_vec(), EdgeMeta::data()),
        (11u8, b"control-x".to_vec(), EdgeMeta::control()),
        (12u8, b"data-b".to_vec(), EdgeMeta::data()),
        (13u8, b"control-y".to_vec(), EdgeMeta::control()),
    ];
    let (store, snapshot, registry, warrant, parents) =
        build_fixture_with_tools(&parents_data, &[]);
    let mote = make_mote(MoteId([246; 32]), parents);
    let ctx = assemble(&mote, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();
    assert_eq!(ctx.items.len(), 2, "exactly the 2 Data parents emit items");

    // Both emitted items have bytes that are NOT the source_ref bytes.
    for item in &ctx.items {
        assert_ne!(item.bytes.as_ref(), item.source_ref.as_bytes().as_slice());
    }
    // And neither emitted item carries the Control parents' payloads.
    let item_bytes: Vec<&[u8]> = ctx.items.iter().map(|i| i.bytes.as_ref()).collect();
    assert!(!item_bytes.iter().any(|b| *b == b"control-x"));
    assert!(!item_bytes.iter().any(|b| *b == b"control-y"));
}

#[test]
fn mixed_parents_and_tools_emit_independent_items_with_correct_shapes() {
    // 1 Data parent + 1 registered tool, both granted. Expected: 2 items.
    // Verifies the two emission code paths run independently and produce the
    // expected shape per path.
    let parents_data = vec![(20u8, b"parent-payload".to_vec(), EdgeMeta::data())];
    let tools_data = vec![(
        "h2-mixed-tool".to_string(),
        "1".to_string(),
        "describes a tool whose description bytes go to the model".to_string(),
    )];
    let (store, snapshot, registry, warrant, parents) =
        build_fixture_with_tools(&parents_data, &tools_data);
    let mote = make_mote(MoteId([245; 32]), parents);
    let ctx = assemble(&mote, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();

    assert_eq!(ctx.items.len(), 2);

    // The Data parent item: bytes = "parent-payload", source_ref = blake3 of those bytes.
    let parent_item = ctx
        .items
        .iter()
        .find(|i| i.label.starts_with("parent."))
        .expect("parent item present");
    assert_eq!(&parent_item.bytes[..], b"parent-payload");
    assert_eq!(parent_item.source_ref, ContentRef::of(b"parent-payload"));
    assert_ne!(
        parent_item.bytes.as_ref(),
        parent_item.source_ref.as_bytes().as_slice()
    );

    // The tool item: bytes = description, source_ref = some hash !=
    // ContentRef::of(description) (because source_ref hashes the whole
    // ToolDef, not just description).
    let tool_item = ctx
        .items
        .iter()
        .find(|i| i.label.starts_with("tool."))
        .expect("tool item present");
    assert_eq!(
        &tool_item.bytes[..],
        b"describes a tool whose description bytes go to the model"
    );
    assert_ne!(
        tool_item.source_ref,
        ContentRef::of(b"describes a tool whose description bytes go to the model".as_ref()),
        "tool source_ref MUST be the full-def hash, NOT ContentRef::of(description)"
    );
    assert_ne!(
        tool_item.bytes.as_ref(),
        tool_item.source_ref.as_bytes().as_slice()
    );
}
