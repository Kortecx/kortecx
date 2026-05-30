//! Throughput / scalability stress harnesses (P4.1 scale & performance validation campaign).
//!
//! These are `#[ignore]`d so the default `cargo test` / CI gate keeps its
//! semantics. Run explicitly, in RELEASE, single-threaded with output:
//!
//! ```text
//! cargo test -p kx-runtime --release --test stress_throughput \
//!     -- --ignored --nocapture --test-threads=1
//! ```
//!
//! - **H1** drives WIDE (one root, N children) and DEEP (chain of N) PURE Mote
//!   DAGs through the REAL single-node runtime surface
//!   (`Scheduler::submit` → `run_pure_mote`) at ramping sizes, printing
//!   motes/sec + wall-clock and asserting exactly-once (`committed_count == N`).
//! - **H2** runs the largest successful DAG twice in independent in-memory
//!   journals and asserts the two canonical digests are byte-identical.
//!
//! The harness ramps sizes and BACKS OFF on the first size that breaches a wall
//! ceiling, reporting the largest size that succeeded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::time::{Duration, Instant};

use kx_content::ContentRef;
use kx_executor::{run_pure_mote, LocalResourceManager, TestMoteExecutor};
use kx_journal::SqliteJournal;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId,
    LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash,
    MOTE_DEF_SCHEMA_VERSION,
};
use kx_projection::Projection;
use kx_runtime::digest_journal;
use kx_scheduler::{LocalPlacement, Scheduler};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use smallvec::SmallVec;
use std::collections::{BTreeMap, BTreeSet};

/// Sizes to ramp through. Stop escalating once a size breaches the wall ceiling.
const SIZES: &[usize] = &[1_000, 5_000, 10_000, 25_000];
/// Per-size wall-clock ceiling. Above this we stop escalating (resource safety).
const WALL_CEILING: Duration = Duration::from_secs(60);

#[derive(Clone, Copy)]
enum Shape {
    /// One parentless root, then (N-1) children all depending on the root.
    Wide,
    /// A chain: node i depends on node i-1.
    Deep,
}

fn mote_def() -> MoteDef {
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: NdClass::Pure,
        config_subset: {
            let mut c = BTreeMap::new();
            c.insert(ConfigKey("k".into()), ConfigVal(vec![1]));
            c
        },
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// A unique PURE Mote at `index` with explicit data-edge parents.
fn mote_at(index: u64, parents: &[MoteId]) -> Mote {
    let mut input = [0u8; 32];
    input[..8].copy_from_slice(&index.to_le_bytes());
    let prefs: SmallVec<[ParentRef; 4]> = parents
        .iter()
        .map(|id| ParentRef {
            parent_id: *id,
            edge: EdgeMeta::data(),
        })
        .collect();
    Mote::new(
        mote_def(),
        InputDataId::from_bytes(input),
        GraphPosition(index.to_le_bytes().to_vec()),
        prefs,
    )
}

/// A permissive PURE warrant (no broker wiring needed for PURE Motes).
fn pure_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope {
            mounts: BTreeMap::new(),
        },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0u8; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
    }
}

/// Build a DAG of `n` PURE Motes in the given shape (topological order).
fn build_dag(n: usize, shape: Shape) -> Vec<Mote> {
    let mut motes: Vec<Mote> = Vec::with_capacity(n);
    let root = mote_at(0, &[]);
    let root_id = root.id;
    motes.push(root);
    for i in 1..n {
        let parents: Vec<MoteId> = match shape {
            Shape::Wide => vec![root_id],
            Shape::Deep => vec![motes[i - 1].id],
        };
        motes.push(mote_at(i as u64, &parents));
    }
    motes
}

/// Submit every Mote to a fresh scheduler+projection, run each PURE Mote to
/// commit over a fresh in-memory journal, return `(committed_count, digest_hex)`.
fn drive(motes: &[Mote], warrant: &WarrantSpec) -> (usize, String) {
    let journal = SqliteJournal::open_in_memory().unwrap();
    let rm = LocalResourceManager::dev_defaults();
    let executor = TestMoteExecutor::deterministic();

    let mut projection = Projection::new();
    let mut scheduler = Scheduler::new(LocalPlacement);
    for m in motes {
        scheduler
            .submit(m.clone(), warrant.clone(), &mut projection)
            .unwrap();
    }
    for m in motes {
        run_pure_mote(m, warrant, &journal, &rm, &executor).unwrap();
    }
    let committed = Projection::from_journal(&journal)
        .unwrap()
        .committed_count();
    let digest = digest_journal(&journal).unwrap().to_hex();
    (committed, digest)
}

fn run_shape(label: &str, shape: Shape) -> usize {
    let warrant = pure_warrant();
    let mut ceiling_reached = 0usize;
    for &n in SIZES {
        let build_start = Instant::now();
        let motes = build_dag(n, shape);
        let build_ms = build_start.elapsed().as_millis();

        let run_start = Instant::now();
        let (committed, _digest) = drive(&motes, &warrant);
        let elapsed = run_start.elapsed();
        let wall_ms = elapsed.as_millis();
        let per_sec = if wall_ms > 0 {
            (n as f64) * 1000.0 / (wall_ms as f64)
        } else {
            f64::INFINITY
        };

        assert_eq!(committed, n, "{label}: exactly-once must hold at n={n}");
        println!(
            "H1 {label}: nodes={n} build_ms={build_ms} run_ms={wall_ms} \
             motes/sec={per_sec:.0} exactly-once=ok",
        );
        ceiling_reached = n;

        if elapsed > WALL_CEILING {
            println!(
                "H1 {label}: wall ceiling {WALL_CEILING:?} breached at n={n}; \
                 stop escalating (ceiling={n})"
            );
            break;
        }
    }
    println!("H1 {label}: ceiling={ceiling_reached}");
    ceiling_reached
}

#[test]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
fn h1_throughput_wide_and_deep() {
    let wide_ceiling = run_shape("WIDE", Shape::Wide);
    let deep_ceiling = run_shape("DEEP", Shape::Deep);
    println!("H1: ceiling WIDE={wide_ceiling} DEEP={deep_ceiling}");
}

#[test]
#[ignore = "stress: run with --release --ignored --nocapture --test-threads=1"]
fn h2_largest_dag_is_byte_reproducible() {
    // Use the largest WIDE size we believe completes under the ceiling. We probe
    // by binary intent: take the largest SIZES entry that ran under ceiling in a
    // quick measurement; for determinism of the harness itself, pick a fixed
    // large size that H1 demonstrated is feasible (10_000) unless 25_000 fits.
    let warrant = pure_warrant();

    // Determine the largest feasible size by timing a single WIDE drive per size.
    let mut chosen = SIZES[0];
    for &n in SIZES {
        let motes = build_dag(n, Shape::Wide);
        let start = Instant::now();
        let (committed, _d) = drive(&motes, &warrant);
        let elapsed = start.elapsed();
        assert_eq!(committed, n);
        chosen = n;
        if elapsed > WALL_CEILING {
            break;
        }
    }

    let motes = build_dag(chosen, Shape::Wide);
    let (c1, digest_a) = drive(&motes, &warrant);
    let (c2, digest_b) = drive(&motes, &warrant);
    assert_eq!(c1, chosen);
    assert_eq!(c2, chosen);
    let identical = digest_a == digest_b;
    assert!(identical, "two independent runs must be byte-identical");
    let prefix: String = digest_a.chars().take(16).collect();
    println!("H2: size={chosen} digests-identical={identical} digest_prefix={prefix}");
}
