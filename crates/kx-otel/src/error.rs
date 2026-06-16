//! [`OtelError`] — the (rare) failure surface of a metrics fold.
//!
//! The only thing that can fail is *reading* the journal (a backend I/O error).
//! Rendering is infallible (a pure string build). Callers treat a fold error as
//! "serve the last good snapshot" — metrics are best-effort observability and must
//! never block or fail the run they observe.

use kx_journal::JournalError;

/// A failure while folding the read-only journal into metrics.
#[derive(Debug, thiserror::Error)]
pub enum OtelError {
    /// The underlying journal read failed. The caller keeps the last good
    /// snapshot rather than surfacing the error to a scraper.
    #[error("journal read failed during metrics fold: {0}")]
    Journal(#[from] JournalError),
}
