//! [`JournalSignal`] — how everything in the serve waits for the journal to advance.
//!
//! Every follower in the serve used to do the same thing: sleep 250 ms, re-read
//! `current_seq()`, and emit if it moved. Six of them, so an idle serve performed sixteen
//! journal reads a second forever and commit→visibility latency was quantized to the poll
//! interval. Now they all await this instead, and an idle serve reads the journal zero
//! times.
//!
//! ## Why the legacy poll survives as a mode rather than being deleted
//!
//! `KX_SERVE_JOURNAL_WATCH=off` returns a signal whose wakeups come from a 250 ms timer —
//! the old behaviour exactly, expressed as a degenerate subscription. Every caller keeps
//! **one** code path, and the difference between the two worlds is a single `match` here
//! rather than a branch in six loops.
//!
//! That buys two things. It is an operator rollback if the seam ever misbehaves in a
//! deployment. And it makes the live A/B honest: one binary, one model, one variable,
//! which is the only way to attribute a latency change to this work rather than to a
//! rebuild. The knob is a lever, not a safety net — nothing falls back to it automatically,
//! and the guard that asserts an idle serve performs no journal reads runs against the
//! **default**, so leaving the poll wired in cannot mask a broken watch.
//!
//! Same precedent as `KX_SERVE_EFFECT_CONCURRENCY=1` restoring the pre-queue sequential
//! batch.

use std::time::Duration;

use kx_journal::JournalSubscription;

/// The legacy cadence, retained only as the `off`-mode wakeup interval. It was chosen to
/// match the CLI/worker idle cadence; nothing about the default path uses it.
pub(crate) const LEGACY_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// How a follower waits for the journal to advance.
pub(crate) enum JournalSignal {
    /// The default: wake when a commit is announced.
    Watch(JournalSubscription),
    /// `KX_SERVE_JOURNAL_WATCH=off`: wake on a timer and re-read, as before.
    Poll(Duration),
}

impl JournalSignal {
    /// Wait for the next opportunity to read.
    ///
    /// Deliberately returns `()` in both modes, and deliberately does **not** hand back a
    /// head. The journal is the authority on what is durable; a watermark is only ever a
    /// lower bound on it, and a caller that treated one as data would be reading a number
    /// that is allowed to lag. So every caller does the same thing after this returns:
    /// read `current_seq()` and emit the range it is owed.
    ///
    /// On a **closed** watch this parks forever instead of returning. A closed watch means
    /// the journal is gone, so no commit can ever be announced again — and returning would
    /// make this arm of the caller's `select!` complete instantly on every pass, busy-
    /// spinning a core for as long as the follower lives. Parking leaves the other arms
    /// (client disconnect, server shutdown) to end the follower, which is what should end
    /// it anyway.
    ///
    /// Unreachable through the serve's own wiring — every follower holds a journal handle,
    /// which keeps the watch alive — but [`JournalSubscription`] is public and `Clone`, so
    /// a holder without a journal handle is expressible. A spin is an expensive way to
    /// discover that.
    pub(crate) async fn changed(&mut self) {
        match self {
            Self::Watch(sub) => {
                if sub.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            Self::Poll(interval) => tokio::time::sleep(*interval).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed watch must park, not return.
    ///
    /// The failure this pins is a busy loop, and a busy loop does not fail a test by
    /// itself — it just makes one slow, or makes production hot. So the assertion is the
    /// inverse of the usual shape: `changed()` must still be pending after a window in
    /// which a returning implementation would have completed thousands of times.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_watch_parks_instead_of_spinning() {
        let watch = kx_journal::JournalWatch::new();
        let mut signal = JournalSignal::Watch(watch.subscribe());
        drop(watch); // the journal is gone; nothing can ever be announced again

        assert!(
            tokio::time::timeout(Duration::from_millis(200), signal.changed())
                .await
                .is_err(),
            "a closed watch returned from `changed()`; in a `select!` loop that is a \
             busy-spin, not a wakeup"
        );
    }

    /// The poll arm still wakes on its interval — otherwise the `off` escape hatch would
    /// park too, and the rollback would be a hang.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_poll_arm_wakes_on_its_interval() {
        let mut signal = JournalSignal::Poll(Duration::from_millis(20));
        assert!(
            tokio::time::timeout(Duration::from_secs(5), signal.changed())
                .await
                .is_ok(),
            "the legacy poll arm never woke"
        );
    }
}
