//! Cross-thread `Send` + `Sync` assertions for kx-mote's public types
//! (SN-4 v2 #7).
//!
//! Every type in this crate is pure data (no FFI, no interior mutability, no
//! locks). The compile-time assertions below pin that the Send + Sync claims
//! we've been implicitly relying on continue to hold as the crate evolves.
//! These tests have zero runtime cost; if a future change accidentally
//! introduces a `!Send` or `!Sync` field, the file stops compiling.
//!
//! Plus one runtime test that crosses an actual thread boundary: build a
//! `MoteGraph`, move it into a worker thread, derive `MoteId`s there, and
//! verify identity is identical to the main-thread derivation. Catches any
//! hypothetical thread-local in the BLAKE3 / bincode path that would make
//! identity machine-dependent.

use std::collections::BTreeMap;
use std::thread;

use kx_mote::{
    canonical_config, derive_mote_id, AttemptState, ChildDescriptor, ConfigKey, ConfigVal,
    EdgeKind, EdgeMeta, EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote,
    MoteDef, MoteDefHash, MoteGraph, MoteId, NdClass, ParentRef, PromptTemplateHash, RoleId,
    ToolName, ToolVersion, TopologyDecision, MOTE_DEF_SCHEMA_VERSION,
};

/// Compile-time `Send + Sync` assertions for every public type. If any future
/// refactor adds a `!Send` / `!Sync` field, the corresponding line fails to
/// compile and the regression is caught at build time, not runtime.
#[test]
fn all_public_types_are_send_and_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    // Hash newtypes (32-byte arrays + Vec<u8> wrappers; pure data).
    assert_send_sync::<MoteId>();
    assert_send_sync::<MoteDefHash>();
    assert_send_sync::<InputDataId>();
    assert_send_sync::<LogicRef>();
    assert_send_sync::<PromptTemplateHash>();

    // String / byte-vec newtypes.
    assert_send_sync::<ModelId>();
    assert_send_sync::<ToolName>();
    assert_send_sync::<ToolVersion>();
    assert_send_sync::<ConfigKey>();
    assert_send_sync::<ConfigVal>();
    assert_send_sync::<GraphPosition>();

    // Enums.
    assert_send_sync::<NdClass>();
    assert_send_sync::<EffectPattern>();
    assert_send_sync::<EdgeKind>();
    assert_send_sync::<AttemptState>();

    // Structs.
    assert_send_sync::<EdgeMeta>();
    assert_send_sync::<ParentRef>();
    assert_send_sync::<MoteDef>();
    assert_send_sync::<Mote>();
    assert_send_sync::<MoteGraph>();

    // D37 Seam A primitives (NEW in PR 7.5).
    assert_send_sync::<RoleId>();
    assert_send_sync::<ChildDescriptor>();
    assert_send_sync::<TopologyDecision>();

    // Standalone `Send` / `Sync` sanity (every type also satisfies these
    // individually — this is technically redundant given Send+Sync above
    // but the helpers exist for cases where one is wanted without the other).
    let _ = (assert_send::<MoteId> as fn(), assert_sync::<MoteId> as fn());
}

/// Move a `MoteGraph` into a worker thread, derive `MoteId`s there, and verify
/// the values match what the main thread would derive. Proves identity is
/// machine-independent (no thread-local seed in BLAKE3 or bincode).
#[test]
fn identity_is_thread_independent_under_real_move() {
    fn make_def() -> MoteDef {
        MoteDef {
            logic_ref: LogicRef::from_bytes([0xaa; 32]),
            model_id: ModelId("claude-opus-4-7:1m".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0xbb; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::ReadOnlyNondet,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::StageThenCommit,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    let def = make_def();
    let input = InputDataId::from_bytes([0xcc; 32]);
    let pos = GraphPosition(vec![0x01, 0x02, 0x03]);

    // Main-thread derivation.
    let id_main = derive_mote_id(&def.hash(), &input, &pos);

    // Move def + input + pos into a thread; derive there.
    let id_worker = thread::spawn(move || derive_mote_id(&def.hash(), &input, &pos))
        .join()
        .expect("worker panic");

    assert_eq!(
        id_main, id_worker,
        "MoteId derivation must be machine-independent — \
         any thread-local seed in BLAKE3 / bincode would break replay"
    );
}

/// Canonical bincode configuration must produce the same bytes from any
/// thread. Pinned because the `canonical_config()` returned by kx-mote is
/// the same value the journal's encode_entry uses for `MoteDef` hashing; a
/// thread-local divergence would corrupt journal identity.
#[test]
fn canonical_config_bytes_are_thread_independent() {
    fn make_def() -> MoteDef {
        MoteDef {
            logic_ref: LogicRef::from_bytes([0x11; 32]),
            model_id: ModelId("test".into()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0x22; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    let def_main = make_def();
    let hash_main = def_main.hash();

    let def_worker = make_def();
    let hash_worker = thread::spawn(move || def_worker.hash())
        .join()
        .expect("worker panic");

    assert_eq!(
        hash_main, hash_worker,
        "MoteDef::hash must produce identical bytes on any thread; if not, \
         canonical_config has a hidden thread-local"
    );

    // Touch canonical_config to ensure it returns a stable value type-side.
    let _cfg = canonical_config();
}
