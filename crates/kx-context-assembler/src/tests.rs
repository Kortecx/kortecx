//! Inline unit tests for kx-context-assembler. Extracted per Rule 3 with
//! bodies unchanged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use bytes::Bytes;
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_journal::{InMemoryJournal, Journal, JournalEntry, ParentEntry};
use kx_mote::{
    derive_mote_id, EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteDefHash, MoteId, NdClass, ParentRef, PromptTemplateHash, ToolName, ToolVersion,
};
use kx_projection::Projection;
use kx_tool_registry::{InMemoryToolRegistry, ToolDef, ToolKind, ToolProvenance, ToolRegistry};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    ToolGrant, ToolRequirement, WarrantSpec,
};
use smallvec::SmallVec;

use super::*;

// -----------------------------------------------------------------
// Test helpers — build a fully-formed Mote, projection, store, registry
// -----------------------------------------------------------------

fn empty_def_hash() -> MoteDefHash {
    MoteDefHash([0; 32])
}

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

fn permissive_req() -> ToolRequirement {
    ToolRequirement {
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
    }
}

/// Build a Committed JournalEntry for the given MoteId. Each test parent
/// must use a distinct `idempotency_key` (dedupe-by-key is on
/// `(idempotency_key, kind=Committed)`); we derive the key from the
/// MoteId's bytes so test parents naturally differ.
fn build_committed_entry(
    mote_id: MoteId,
    result_ref: ContentRef,
    parents: SmallVec<[ParentEntry; 4]>,
) -> JournalEntry {
    JournalEntry::Committed {
        mote_id,
        idempotency_key: mote_id.0,
        seq: 0,
        nondeterminism: NdClass::Pure,
        result_ref,
        parents,
        warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        mote_def_hash: empty_def_hash(),
    }
}

/// Make a Mote with given id, parents, and graph_position. The MoteDef
/// is a minimal placeholder; the assembler only reads `mote.parents`.
fn make_mote(mote_id: MoteId, parents: SmallVec<[ParentRef; 4]>, position: GraphPosition) -> Mote {
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
        graph_position: position,
        parents,
    }
}

fn mid(seed: u8) -> MoteId {
    MoteId([seed; 32])
}

// -----------------------------------------------------------------
// Happy path: 2 parents + 1 tool
// -----------------------------------------------------------------

#[test]
fn assemble_two_parents_one_tool() {
    // Build content store with 2 parents' bytes.
    let store = InMemoryContentStore::new();
    let parent_a_bytes = b"output of parent A".to_vec();
    let parent_b_bytes = b"output of parent B".to_vec();
    let parent_a_ref = store.put(&parent_a_bytes).unwrap();
    let parent_b_ref = store.put(&parent_b_bytes).unwrap();

    // Build a journal with the two parent Motes committed.
    let parent_a_id = mid(1);
    let parent_b_id = mid(2);
    let journal = InMemoryJournal::new();
    let e_a = build_committed_entry(parent_a_id, parent_a_ref, SmallVec::new());
    let e_b = build_committed_entry(parent_b_id, parent_b_ref, SmallVec::new());
    let _ = journal.append(e_a).unwrap();
    let _ = journal.append(e_b).unwrap();

    // Fold into projection + snapshot.
    let proj = Projection::from_journal(&journal).unwrap();
    let snapshot = proj.snapshot();

    // Build the registry with one tool granted to the warrant.
    let mut registry = InMemoryToolRegistry::new();
    let tool = ToolDef {
        tool_id: ToolName("fs-read".into()),
        tool_version: ToolVersion("1".into()),
        kind: ToolKind::Builtin,
        required_capability: permissive_req(),
        description: "reads files".into(),
        idempotency_class: kx_tool_registry::IdempotencyClass::Readback,
    };
    let _ = registry
        .register(
            tool.clone(),
            ToolProvenance::HumanAuthored {
                author: "ops".into(),
            },
        )
        .unwrap();

    // Build the child Mote referencing both parents on Data edges.
    let mut warrant = permissive_warrant();
    warrant.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: tool.tool_id.clone(),
        tool_version: tool.tool_version.clone(),
    }]);
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
        ParentRef {
            parent_id: parent_a_id,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: parent_b_id,
            edge: EdgeMeta::data(),
        },
    ]);
    let child_id = derive_mote_id(
        &empty_def_hash(),
        &InputDataId([3; 32]),
        &GraphPosition(vec![0, 0]),
    );
    let child = make_mote(child_id, parents, GraphPosition(vec![0, 0]));

    // Assemble.
    let ctx = assemble(&child, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap();

    // 3 items: 2 parents + 1 tool.
    assert_eq!(ctx.items.len(), 3);
    // Parents come first (sorted by MoteId bytes — A < B because 1 < 2).
    assert!(ctx.items[0].label.starts_with("parent."));
    assert!(ctx.items[1].label.starts_with("parent."));
    // Tool comes after.
    assert_eq!(ctx.items[2].label, "tool.fs-read@1");
    // Parent bytes are resolved content (NEVER hashes).
    assert_eq!(&ctx.items[0].bytes[..], parent_a_bytes);
    assert_eq!(&ctx.items[1].bytes[..], parent_b_bytes);
    // Tool item carries the description.
    assert_eq!(&ctx.items[2].bytes[..], b"reads files");
}

// -----------------------------------------------------------------
// Deterministic parent ordering by MoteId bytes
// -----------------------------------------------------------------

#[test]
fn parent_order_is_deterministic_by_mote_id_bytes() {
    let store = InMemoryContentStore::new();
    let r_5 = store.put(b"five").unwrap();
    let r_3 = store.put(b"three").unwrap();
    let r_7 = store.put(b"seven").unwrap();
    let id_5 = mid(5);
    let id_3 = mid(3);
    let id_7 = mid(7);

    let journal = InMemoryJournal::new();
    let e5 = build_committed_entry(id_5, r_5, SmallVec::new());
    let e3 = build_committed_entry(id_3, r_3, SmallVec::new());
    let e7 = build_committed_entry(id_7, r_7, SmallVec::new());
    // Append in arbitrary order — sort happens in the assembler.
    let _ = journal.append(e5).unwrap();
    let _ = journal.append(e3).unwrap();
    let _ = journal.append(e7).unwrap();

    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();
    let registry = InMemoryToolRegistry::new();

    // Declare parents in non-sorted order — assembler must still sort.
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
        ParentRef {
            parent_id: id_7,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: id_3,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: id_5,
            edge: EdgeMeta::data(),
        },
    ]);
    let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

    let ctx = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        usize::MAX,
    )
    .unwrap();

    assert_eq!(ctx.items.len(), 3);
    // Sorted: 3 < 5 < 7 in MoteId byte order.
    assert_eq!(&ctx.items[0].bytes[..], b"three");
    assert_eq!(&ctx.items[1].bytes[..], b"five");
    assert_eq!(&ctx.items[2].bytes[..], b"seven");
}

// -----------------------------------------------------------------
// Control edges contribute NO content
// -----------------------------------------------------------------

#[test]
fn control_edges_skipped() {
    let store = InMemoryContentStore::new();
    let r_data = store.put(b"data").unwrap();
    let r_ctrl = store.put(b"control output").unwrap();
    let id_d = mid(1);
    let id_c = mid(2);

    let journal = InMemoryJournal::new();
    let ed = build_committed_entry(id_d, r_data, SmallVec::new());
    let ec = build_committed_entry(id_c, r_ctrl, SmallVec::new());
    let _ = journal.append(ed).unwrap();
    let _ = journal.append(ec).unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let registry = InMemoryToolRegistry::new();
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![
        ParentRef {
            parent_id: id_d,
            edge: EdgeMeta::data(),
        },
        ParentRef {
            parent_id: id_c,
            edge: EdgeMeta::control(),
        },
    ]);
    let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

    let ctx = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        usize::MAX,
    )
    .unwrap();

    // Only the Data parent's bytes — Control is skipped.
    assert_eq!(ctx.items.len(), 1);
    assert_eq!(&ctx.items[0].bytes[..], b"data");
}

// -----------------------------------------------------------------
// Error: parent not committed
// -----------------------------------------------------------------

#[test]
fn missing_committed_parent_errors() {
    let store = InMemoryContentStore::new();
    let snapshot = Projection::new().snapshot(); // empty
    let registry = InMemoryToolRegistry::new();
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
        parent_id: mid(99),
        edge: EdgeMeta::data(),
    }]);
    let child = make_mote(mid(0), parents, GraphPosition(vec![0]));

    let err = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        usize::MAX,
    )
    .unwrap_err();

    match err {
        AssemblyError::UpstreamNotCommitted { parent_mote_id } => {
            assert_eq!(parent_mote_id, mid(99));
        }
        other => panic!("expected UpstreamNotCommitted, got {other:?}"),
    }
}

// -----------------------------------------------------------------
// Error: content store miss
// -----------------------------------------------------------------

#[test]
fn content_store_miss_errors() {
    // Commit a parent in the journal but DON'T put its bytes in the store.
    let store = InMemoryContentStore::new();
    let fake_ref = ContentRef::from_bytes([42; 32]);
    let id_a = mid(1);

    let journal = InMemoryJournal::new();
    let e = build_committed_entry(id_a, fake_ref, SmallVec::new());
    let _ = journal.append(e).unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let registry = InMemoryToolRegistry::new();
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
        parent_id: id_a,
        edge: EdgeMeta::data(),
    }]);
    let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

    let err = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        usize::MAX,
    )
    .unwrap_err();

    assert!(matches!(err, AssemblyError::ContentStoreMiss { .. }));
}

// -----------------------------------------------------------------
// Error: tool not resolvable
// -----------------------------------------------------------------

#[test]
fn tool_not_resolvable_errors() {
    let store = InMemoryContentStore::new();
    let snapshot = Projection::new().snapshot();
    let registry = InMemoryToolRegistry::new(); // empty — no tools

    let mut warrant = permissive_warrant();
    warrant.tool_grants = BTreeSet::from([ToolGrant {
        tool_id: ToolName("nope".into()),
        tool_version: ToolVersion("1".into()),
    }]);

    let child = make_mote(mid(0), SmallVec::new(), GraphPosition(vec![0]));

    let err = assemble(&child, &warrant, &snapshot, &store, &registry, usize::MAX).unwrap_err();

    match err {
        AssemblyError::ToolNotResolvable { grant, .. } => {
            assert_eq!(grant.tool_id.0, "nope");
        }
        other => panic!("expected ToolNotResolvable, got {other:?}"),
    }
}

// -----------------------------------------------------------------
// Overflow: closure size exceeds window
// -----------------------------------------------------------------

#[test]
fn overflow_decision_required_when_window_too_small() {
    let store = InMemoryContentStore::new();
    let big_payload = vec![b'x'; 4096];
    let r = store.put(&big_payload).unwrap();
    let id = mid(1);

    let journal = InMemoryJournal::new();
    let e = build_committed_entry(id, r, SmallVec::new());
    let _ = journal.append(e).unwrap();
    let snapshot = Projection::from_journal(&journal).unwrap().snapshot();

    let registry = InMemoryToolRegistry::new();
    let parents: SmallVec<[ParentRef; 4]> = SmallVec::from_vec(vec![ParentRef {
        parent_id: id,
        edge: EdgeMeta::data(),
    }]);
    let child = make_mote(mid(9), parents, GraphPosition(vec![0]));

    // window_bytes = 100 — far less than 4096.
    let err = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        100,
    )
    .unwrap_err();

    match err {
        AssemblyError::OverflowDecisionRequired {
            closure_size_bytes,
            window_bytes,
        } => {
            assert_eq!(closure_size_bytes, 4096);
            assert_eq!(window_bytes, 100);
        }
        other => panic!("expected OverflowDecisionRequired, got {other:?}"),
    }
}

// -----------------------------------------------------------------
// Empty assembly: no parents + no tool grants
// -----------------------------------------------------------------

#[test]
fn empty_assembly_is_empty_context() {
    let store = InMemoryContentStore::new();
    let snapshot = Projection::new().snapshot();
    let registry = InMemoryToolRegistry::new();
    let child = make_mote(mid(0), SmallVec::new(), GraphPosition(vec![0]));
    let ctx = assemble(
        &child,
        &permissive_warrant(),
        &snapshot,
        &store,
        &registry,
        usize::MAX,
    )
    .unwrap();
    assert!(ctx.is_empty());
    assert_eq!(ctx.total_bytes(), 0);
    assert_eq!(ctx.len(), 0);
}

// -----------------------------------------------------------------
// content_ref is byte-deterministic
// -----------------------------------------------------------------

#[test]
fn content_ref_is_deterministic() {
    let ctx = AssembledContext {
        items: vec![
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
        ],
    };
    assert_eq!(ctx.content_ref(), ctx.content_ref());
}

#[test]
fn content_ref_ignores_labels_and_source_refs() {
    let ctx_a = AssembledContext {
        items: vec![AssembledItem {
            label: "label-a".into(),
            bytes: Bytes::from_static(b"same bytes"),
            source_ref: ContentRef::from_bytes([1; 32]),
        }],
    };
    let ctx_b = AssembledContext {
        items: vec![AssembledItem {
            label: "label-b".into(),
            bytes: Bytes::from_static(b"same bytes"),
            source_ref: ContentRef::from_bytes([99; 32]),
        }],
    };
    // Same bytes, different labels/source_refs → same content_ref.
    assert_eq!(ctx_a.content_ref(), ctx_b.content_ref());
}

#[test]
fn content_ref_changes_with_bytes() {
    let ctx_a = AssembledContext {
        items: vec![AssembledItem {
            label: "a".into(),
            bytes: Bytes::from_static(b"alpha"),
            source_ref: ContentRef::from_bytes([0; 32]),
        }],
    };
    let ctx_b = AssembledContext {
        items: vec![AssembledItem {
            label: "a".into(),
            bytes: Bytes::from_static(b"alphb"),
            source_ref: ContentRef::from_bytes([0; 32]),
        }],
    };
    assert_ne!(ctx_a.content_ref(), ctx_b.content_ref());
}
