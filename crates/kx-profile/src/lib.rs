//! kortecx runtime profiling harness (Golden Rule 10).
//!
//! The **public half** of the measure-and-persist discipline: an FFI-free
//! library + bin that hosts an in-process gateway, measures warm-up +
//! submit→Committed latency, and captures an **environment-labelled, schema-1
//! JSON** [`Report`]. The captured numbers are a corpus trend record that lives
//! ONLY in the private repo (`docs/benchmarks/`, gitignored on OSS — SN-2); the
//! harness here is the reusable, public measurement tool.
//!
//! Design constraints (Golden Rule 10 + the workspace invariants):
//! - **FFI-free** — the default gateway closure has no llama.cpp, so any
//!   contributor can profile their box without a C++ toolchain.
//! - **No `build.rs`** — the environment (`git_sha`, toolchain, host, cpu) is
//!   captured at run time, so the compiled artifact carries no volatile bytes
//!   and the `check-reproducible` (I1.c) byte-determinism gate is unaffected.
//! - **Off the hot path** — all timing is at the client/dispatch boundary,
//!   never inside the sole-writer commit path or the digest fold.

// Test modules host an in-process gateway + assert on captured values; the
// safety lints (deny in library code) are relaxed for tests per Rule 3.
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)
)]

pub mod content_spikes;
pub mod env;
pub mod error;
pub mod react_spikes;
pub mod report;
pub mod spikes;

pub use content_spikes::ContentSamples;
pub use env::{capture_git_sha, Environment};
pub use error::ProfileError;
pub use react_spikes::ReactSamples;
pub use report::{Metric, MetricKind, Report, SCHEMA_VERSION};
pub use spikes::{percentile, LatencySamples};
