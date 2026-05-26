//! Resolve a Mote's `logic_ref` to a runnable body path on disk.
//!
//! Production callers configure the executor with an `Arc<dyn BodyResolver>`;
//! at run time the executor calls `resolve(&logic_ref)` to materialize the
//! body bytes (from a `ContentStore`) into a tempfile + chmod +x + return
//! the path. The caller execvps the path; the spawned child inherits
//! through to the body binary.
//!
//! The trait exists because `ContentStore` has an associated `Payload`
//! type that makes `dyn ContentStore` unworkable. `BodyResolver` is
//! object-safe (no generics, no associated types) so executors can hold
//! `Arc<dyn BodyResolver>` while concrete impls like
//! `ContentStoreBodyResolver<S: ContentStore>` carry the generic.

use std::io::Write;
use std::path::Path;

use kx_content::{ContentRef, ContentStore};
use kx_mote::LogicRef;
use thiserror::Error;

/// Errors from resolving a Mote's `logic_ref` to a runnable body path.
#[derive(Debug, Error)]
pub enum BodyResolverError {
    /// The configured `ContentStore` doesn't have the bytes the `logic_ref`
    /// points at. Production callers should pre-populate the store before
    /// dispatching the Mote.
    #[error("logic_ref content not in store: {hex}")]
    NotInStore {
        /// Lowercase-hex of the logic_ref bytes (32 bytes × 2 = 64 chars).
        hex: String,
    },
    /// Filesystem error materializing the body bytes (tempfile creation,
    /// write, or flush).
    #[error("materialize body: {0}")]
    Io(String),
    /// `chmod +x` on the materialized tempfile failed.
    #[error("chmod +x on body path failed: {0}")]
    ChmodFailed(String),
}

/// Resolve a Mote's `logic_ref` to a runnable body path. Object-safe +
/// `Send + Sync` so callers hold `Arc<dyn BodyResolver>`.
pub trait BodyResolver: Send + Sync {
    /// Resolve `logic_ref` to a `MaterializedBody` whose `path()` is safe
    /// to execvp. The caller MUST keep the `MaterializedBody` alive until
    /// the spawned process has been reaped — its `Drop` removes the
    /// underlying tempfile.
    ///
    /// # Errors
    ///
    /// Returns `BodyResolverError` variants when the content store lookup
    /// fails (`NotInStore`), the tempfile write fails (`Io`), or the
    /// permissions update fails (`ChmodFailed`).
    fn resolve(&self, logic_ref: &LogicRef) -> Result<MaterializedBody, BodyResolverError>;
}

/// A body binary materialized to a temporary file. `Drop` removes the
/// file. Callers `execvp` `path()` then keep `Self` alive until waitpid
/// reaps the child.
pub struct MaterializedBody {
    file: tempfile::NamedTempFile,
}

impl MaterializedBody {
    /// The path on disk the executor can `execvp`. Lives until `Drop`
    /// removes the file.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

impl std::fmt::Debug for MaterializedBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaterializedBody")
            .field("path", &self.file.path())
            .finish()
    }
}

/// `BodyResolver` impl backed by a `ContentStore`. Generic over the
/// concrete `ContentStore` since `ContentStore::Payload` is an associated
/// type that prevents `dyn ContentStore` directly.
pub struct ContentStoreBodyResolver<S: ContentStore> {
    store: S,
}

impl<S: ContentStore> ContentStoreBodyResolver<S> {
    /// Construct a resolver wrapping the given `ContentStore`.
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> std::fmt::Debug for ContentStoreBodyResolver<S>
where
    S: ContentStore,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentStoreBodyResolver").finish()
    }
}

impl<S> BodyResolver for ContentStoreBodyResolver<S>
where
    S: ContentStore + Send + Sync,
    S::Payload: Send + Sync,
{
    fn resolve(&self, logic_ref: &LogicRef) -> Result<MaterializedBody, BodyResolverError> {
        let content_ref = ContentRef::from_bytes(*logic_ref.as_bytes());
        let bytes_handle =
            self.store
                .get(&content_ref)
                .map_err(|_| BodyResolverError::NotInStore {
                    hex: hex_of(logic_ref.as_bytes()),
                })?;

        let mut file = tempfile::NamedTempFile::new()
            .map_err(|e| BodyResolverError::Io(format!("tempfile: {e}")))?;
        file.write_all(&bytes_handle)
            .map_err(|e| BodyResolverError::Io(format!("write: {e}")))?;
        file.flush()
            .map_err(|e| BodyResolverError::Io(format!("flush: {e}")))?;

        // chmod +x — required for execvp to honor the binary as
        // executable. On Unix this is `Permissions::from_mode(0o755)`;
        // production callers may want stricter modes (0o700) if the
        // tempfile dir is shared with other UIDs.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o755))
                .map_err(|e| BodyResolverError::ChmodFailed(e.to_string()))?;
        }

        Ok(MaterializedBody { file })
    }
}

/// Lowercase-hex encode a 32-byte hash for error messages.
fn hex_of(bytes: &[u8; 32]) -> String {
    const NIBBLES: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(NIBBLES[(byte >> 4) as usize] as char);
        out.push(NIBBLES[(byte & 0x0F) as usize] as char);
    }
    out
}
