// SPDX-License-Identifier: Apache-2.0
//! `kx-otel` — OFF-TRUTH-PATH observability metrics (W1a / T-OBS2).
//!
//! Folds the **read-only journal** into RED metrics — **R**ate (runs/commits),
//! **E**rrors (failures by reason), and (host-supplied) **D**uration — and renders
//! the Prometheus text exposition format. Everything here is derived from durable
//! committed facts; nothing is an identity or digest input, so turning metrics on
//! changes only what is *observed* (the canonical product digest `7d22d4bd…` is
//! byte-unchanged with metrics on, off, or scraped — the `kx-audit` posture).
//!
//! ## Design constraints (the workspace invariants)
//! - **FFI-free** — kx-otel reads journal facts; it never dispatches inference, so
//!   the dep-wall keeps it off the llama.cpp closure (any contributor can build it).
//! - **No `build.rs`** — no compile-time environment capture, so the artifact is
//!   byte-deterministic and the `check-reproducible` (I1.c) gate is unaffected.
//! - **Off the hot path / fail-open** — the fold is incremental
//!   ([`MetricsState::fold_from`]) and runs on a background tick; a scrape renders a
//!   cached [`MetricsState`] snapshot ([`MetricsHandle::render`]) and never scans
//!   the journal, so scrape latency is independent of journal size.
//! - **Read-only by type** — the handle holds an [`kx_gateway_core::JournalReader`]
//!   (no `append`); a write cannot type-check.

#![forbid(unsafe_code)]
// Tests assert on rendered/folded values via unwrap and build fixture entries
// from small seqs (`seq as u8` byte patterns); the safety/pedantic lints (deny in
// library code) are relaxed for tests per the workspace Rule-3 convention.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::cast_possible_truncation,
        clippy::default_trait_access,
        clippy::similar_names
    )
)]

mod error;
mod fold;
mod render;

use std::sync::{Arc, Mutex, PoisonError};

use kx_gateway_core::JournalReader;

pub use error::OtelError;
pub use fold::{MetricsState, FAILURE_REASON_COUNT, FAILURE_REASON_LABELS};
pub use render::{render, BuildInfo, LatencySummary};

/// A shareable metrics handle: the read-only journal seam + a cached
/// [`MetricsState`] snapshot.
///
/// Lifecycle (mirrors the gateway's telemetry ledger): a background task calls
/// [`Self::refresh`] on a tick to fold the journal tail into the cached snapshot;
/// the `/metrics` scrape calls [`Self::render`] which serves the cached snapshot
/// (plus optional host-supplied latency) **without** folding — so a scrape is
/// fast regardless of journal size. Cloneable: the inner state + reader are
/// `Arc`-shared, so the tick and the scrape see the same cache.
#[derive(Clone)]
pub struct MetricsHandle {
    state: Arc<Mutex<MetricsState>>,
    reader: Arc<dyn JournalReader>,
    build: BuildInfo,
}

impl MetricsHandle {
    /// Build a handle over a read-only journal seam, labelling renders with `build`.
    #[must_use]
    pub fn new(reader: Arc<dyn JournalReader>, build: BuildInfo) -> Self {
        Self {
            state: Arc::new(Mutex::new(MetricsState::new())),
            reader,
            build,
        }
    }

    /// Fold the journal tail into the cached snapshot (incremental + idempotent).
    /// Call this on a background tick. A poisoned cache lock is recovered (the
    /// inner state is plain counters — a panicked holder leaves it usable).
    ///
    /// # Errors
    /// Returns [`OtelError::Journal`] if the underlying journal read fails; the
    /// cached snapshot is left intact so a scrape still serves the last good data.
    pub fn refresh(&self) -> Result<(), OtelError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.fold_from(self.reader.as_ref())
    }

    /// A clone of the current cached snapshot (no journal read).
    #[must_use]
    pub fn snapshot(&self) -> MetricsState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Render the `/metrics` body from the cached snapshot plus optional
    /// host-supplied recent-window latency. Does **not** fold — serve the cache.
    #[must_use]
    pub fn render(&self, latency: Option<&LatencySummary>) -> String {
        let snapshot = self.snapshot();
        render(&snapshot, &self.build, latency)
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use kx_journal::{JournalEntry, JournalError};

    use super::*;

    struct EmptyReader;
    impl JournalReader for EmptyReader {
        fn read_entries_by_seq(
            &self,
            _range: Range<u64>,
        ) -> Result<Box<dyn Iterator<Item = JournalEntry> + '_>, JournalError> {
            Ok(Box::new(std::iter::empty()))
        }
        fn current_seq(&self) -> Result<u64, JournalError> {
            Ok(0)
        }
    }

    #[test]
    fn handle_renders_empty_state() {
        let handle = MetricsHandle::new(Arc::new(EmptyReader), BuildInfo { version: "0.0.0" });
        handle.refresh().unwrap();
        let body = handle.render(None);
        assert!(body.contains("kortecx_up 1"));
        assert!(body.contains("kortecx_motes_committed_total 0"));
        assert_eq!(handle.snapshot(), MetricsState::new());
    }
}
