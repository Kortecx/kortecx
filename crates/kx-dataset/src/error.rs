//! [`DataError`] — the data-layer error vocabulary.

use thiserror::Error;

/// An error from a [`crate::DataStore`] operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DataError {
    /// No payload is stored at the requested [`kx_content::ContentRef`]. Because
    /// the data layer is journal-authoritative (a cache/projection of committed
    /// content), a miss is recoverable by re-folding committed content.
    #[error("no payload stored at the requested content ref")]
    NotFound,

    /// The store's lock was poisoned by a panic in another thread.
    #[error("data store lock poisoned")]
    Poisoned,
}
