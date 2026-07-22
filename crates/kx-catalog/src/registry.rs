// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The catalog seam (M7.1): the backend-agnostic [`CatalogRegistry`] trait, its
//! [`RegistrationOutcome`] / [`CatalogError`] vocabulary, and an `Arc` blanket
//! impl. The in-memory reference backend is [`crate::InMemoryCatalog`].
//!
//! Registration is **content-addressed, idempotent, and immutable**: keyed by
//! the entry's [`TaskSignatureHash`], a re-register of a byte-identical entry is
//! a no-op, and a re-register of a *different* entry at the same hash is refused
//! ([`CatalogError::ImmutabilityConflict`]) rather than silently overwritten
//! (asymmetric strictness). The trait carries no storage-substrate assumption —
//! a persistent / cloud backend is a later impl behind it, exactly as
//! `kx_content::ContentStore`.

use std::sync::Arc;

use crate::entry::SignatureEntry;
use crate::signature::TaskSignatureHash;

/// The result of [`CatalogRegistry::register_signature`] — distinguishes a fresh
/// insert from an idempotent no-op so callers observe idempotency without a
/// second lookup.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegistrationOutcome {
    /// First registration of this signature hash.
    Inserted(TaskSignatureHash),
    /// A byte-identical entry was already present — no-op (idempotent).
    AlreadyPresent(TaskSignatureHash),
}

impl RegistrationOutcome {
    /// The signature hash this outcome refers to.
    #[must_use]
    pub const fn hash(&self) -> TaskSignatureHash {
        match self {
            Self::Inserted(h) | Self::AlreadyPresent(h) => *h,
        }
    }

    /// `true` iff this was a fresh insert (not an idempotent no-op).
    #[must_use]
    pub const fn is_inserted(&self) -> bool {
        matches!(self, Self::Inserted(_))
    }
}

/// A catalog registration failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    /// A DIFFERENT entry already exists at this signature hash. Registration is
    /// immutable: the same hash MUST mean the same bytes. A mismatch is a caller
    /// bug (or a hash-collision attack surface) — refuse loudly rather than
    /// overwrite a registered fact. Carries the conflicting hash (hex).
    #[error("immutable catalog conflict at task_signature_hash {0}")]
    ImmutabilityConflict(String),
    /// A durable-backend storage failure (SQLite I/O, a corrupt row, or a
    /// schema-version mismatch on open). Owned `String` (not `#[from]
    /// rusqlite::Error`) so the enum stays `Clone + PartialEq + Eq`.
    #[error("catalog storage error: {0}")]
    Storage(String),
}

/// The catalog seam — backend-agnostic (in-memory now; SQLite / cloud later
/// behind the same trait). Authoritative for WHAT recipes exist (a separate
/// truth from the journal); it never writes the journal.
pub trait CatalogRegistry {
    /// Register a signature entry, content-addressed by its
    /// [`TaskSignatureHash`]. Idempotent + immutable:
    /// - hash absent → insert, return [`RegistrationOutcome::Inserted`];
    /// - present and byte-identical → no-op, [`RegistrationOutcome::AlreadyPresent`];
    /// - present but different bytes → [`CatalogError::ImmutabilityConflict`].
    ///
    /// # Errors
    ///
    /// Returns [`CatalogError::ImmutabilityConflict`] when a different entry is
    /// already registered under the same signature hash.
    fn register_signature(
        &self,
        entry: SignatureEntry,
    ) -> Result<RegistrationOutcome, CatalogError>;

    /// Exact lookup by content hash; `None` if absent. O(log n).
    fn lookup(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry>;

    /// Get by hash (alias of [`CatalogRegistry::lookup`], named per the
    /// `GetSignature` API surface).
    fn get_signature(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry> {
        self.lookup(hash)
    }

    /// Enumerate every registered entry in deterministic (hash) order.
    fn list_signatures<'a>(&'a self) -> Box<dyn Iterator<Item = SignatureEntry> + 'a>;

    /// Count of registered entries.
    fn len(&self) -> usize;

    /// `true` when no signatures are registered.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<R: CatalogRegistry + ?Sized> CatalogRegistry for Arc<R> {
    fn register_signature(
        &self,
        entry: SignatureEntry,
    ) -> Result<RegistrationOutcome, CatalogError> {
        (**self).register_signature(entry)
    }

    fn lookup(&self, hash: &TaskSignatureHash) -> Option<SignatureEntry> {
        (**self).lookup(hash)
    }

    fn list_signatures<'a>(&'a self) -> Box<dyn Iterator<Item = SignatureEntry> + 'a> {
        (**self).list_signatures()
    }

    fn len(&self) -> usize {
        (**self).len()
    }
}
