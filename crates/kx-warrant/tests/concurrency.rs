// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group
// is also allowed here — tests routinely do things pedantic flags (small-int
// casts on byte seeds, helper-fn definitions after let-bindings, etc.) that
// would be needless friction to refactor.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Concurrency tests for `kx-warrant` (SN-4 v2 #7 — concurrency mandate; D30).
//!
//! - Compile-time `Send + Sync` assertions over the full public-type set.
//! - 4-thread thread-independence of `intersect` (Arc<>'d inputs, identical
//!   outcomes).
//! - 4-thread byte-identity of `warrant_ref_of` (no clock, no thread-local).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use kx_content::ContentRef;
use kx_mote::{ModelId, ToolName, ToolVersion};
use kx_warrant::{
    intersect, warrant_ref_of, ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass,
    NarrowingError, NetScope, ResourceCeiling, Role, ToolDenied, ToolGrant, ToolRequirement,
    WarrantField, WarrantSpec,
};

// ---------------------------------------------------------------------------
// Compile-time Send + Sync assertions
// ---------------------------------------------------------------------------

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn public_types_are_send_and_sync() {
    // Small enums
    assert_send_sync::<MoteClass>();
    assert_send_sync::<ExecutorClass>();
    assert_send_sync::<FsMode>();
    assert_send_sync::<WarrantField>();

    // Composite axes
    assert_send_sync::<Host>();
    assert_send_sync::<FsScope>();
    assert_send_sync::<NetScope>();
    assert_send_sync::<ToolGrant>();
    assert_send_sync::<ModelRoute>();
    assert_send_sync::<ResourceCeiling>();
    assert_send_sync::<WarrantSpec>();
    assert_send_sync::<Role>();
    assert_send_sync::<ToolRequirement>();

    // Errors
    assert_send_sync::<NarrowingError>();
    assert_send_sync::<ToolDenied>();
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn permissive_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::from([
                (PathBuf::from("/input"), FsMode::ReadOnly),
                (PathBuf::from("/output"), FsMode::ReadWrite),
            ]),
        },
        net_scope: NetScope::EgressAllowlist(BTreeSet::from([Host("api.example.com:443".into())])),
        syscall_profile_ref: ContentRef::from_bytes([7; 32]),
        tool_grants: BTreeSet::from([
            ToolGrant {
                tool_id: ToolName("fs-read".into()),
                tool_version: ToolVersion("1".into()),
            },
            ToolGrant {
                tool_id: ToolName("http-get".into()),
                tool_version: ToolVersion("2".into()),
            },
        ]),
        model_route: ModelRoute {
            model_id: ModelId("gpt-4".into()),
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
        ..Default::default()
    }
}

fn tightening_role() -> Role {
    let mut spec = permissive_warrant();
    spec.resource_ceiling.cpu_milli = 500;
    spec.model_route.max_calls = 3;
    Role {
        name: "tighten".into(),
        version: 1,
        spec,
        description: String::new(),
    }
}

// ---------------------------------------------------------------------------
// 4-thread thread-independence of `intersect`
// ---------------------------------------------------------------------------

/// 4 threads each compute `intersect(parent, role)` against identical inputs
/// (via `Arc<>`). The result must be byte-identical across threads — pins the
/// "no thread-local seed in BLAKE3/bincode" contract that machine-independent
/// replay depends on.
#[test]
fn intersect_is_thread_independent_under_real_move() {
    let parent = Arc::new(permissive_warrant());
    let role = Arc::new(tightening_role());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let p = Arc::clone(&parent);
            let r = Arc::clone(&role);
            thread::spawn(move || intersect(&p, &r).expect("ok"))
        })
        .collect();

    let mut results = Vec::with_capacity(4);
    for h in handles {
        results.push(h.join().expect("worker did not panic"));
    }

    // All four threads produced byte-identical warrants.
    let first = &results[0];
    for r in &results[1..] {
        assert_eq!(first, r, "intersect must be thread-independent");
    }
}

// ---------------------------------------------------------------------------
// 4-thread byte-identity of `warrant_ref_of`
// ---------------------------------------------------------------------------

/// 4 threads each compute `warrant_ref_of(spec)` against an Arc<>'d spec. The
/// resulting `ContentRef`s must be byte-identical — pins the deterministic
/// content-addressing contract that journal recovery depends on.
#[test]
fn warrant_ref_of_is_thread_independent() {
    let spec = Arc::new(permissive_warrant());

    let handles: Vec<_> = (0..4)
        .map(|_| {
            let s = Arc::clone(&spec);
            thread::spawn(move || warrant_ref_of(&s))
        })
        .collect();

    let mut refs = Vec::with_capacity(4);
    for h in handles {
        refs.push(h.join().expect("worker did not panic"));
    }

    let first = refs[0];
    for r in &refs[1..] {
        assert_eq!(&first, r, "warrant_ref_of must be thread-independent");
    }
}
