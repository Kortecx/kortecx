//! The local-filesystem [`ContentStore`] backend.
//!
//! Atomicity: write-to-temp + POSIX atomic rename. The temp file lives in the same directory
//! as its final destination so the rename is filesystem-local and atomic per POSIX. A writer
//! that dies between the temp-file creation and the rename leaves the temp file behind — it
//! is reclaimed by the operating-system temp-file cleanup or by an explicit retention pass;
//! it never appears at the target ref because the rename never happened.
//!
//! Zero-copy reads: this backend reads the file's bytes into a [`bytes::Bytes`] buffer. The
//! `Bytes` wrapper enables zero-copy slicing and reference-counted sharing for downstream
//! consumers. (True mmap-based zero-copy from disk is a future enhancement that would require
//! the `memmap2` crate; the v0.1 implementation reads through a single `Vec<u8>` allocation.)

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::{ContentRef, ContentStore, NotFound, StoreError};

/// A [`ContentStore`] backed by a single local filesystem directory.
///
/// Each object is a regular file at `root/<hex_hash>`, where the hex_hash is the lowercase
/// 64-character hex encoding of the BLAKE3 hash of the file's bytes (per
/// [`ContentRef::to_hex`]). The directory contains no other files; an enumeration scan
/// parses each filename back into a [`ContentRef`].
///
/// Construction creates the root directory if it doesn't exist; failures bubble up via
/// [`StoreError::Io`].
///
/// # Examples
///
/// ```
/// use kx_content::{ContentStore, LocalFsContentStore};
/// use tempfile::TempDir;
///
/// let tmp = TempDir::new().unwrap();
/// let store = LocalFsContentStore::open(tmp.path()).unwrap();
///
/// let r = store.put(b"payload bytes").unwrap();
/// assert!(store.contains(&r));
/// assert_eq!(&*store.get(&r).unwrap(), b"payload bytes");
///
/// store.delete(&r).unwrap();
/// assert!(!store.contains(&r));
/// ```
#[derive(Debug, Clone)]
pub struct LocalFsContentStore {
    root: PathBuf,
}

impl LocalFsContentStore {
    /// Open or create a `LocalFsContentStore` rooted at `root`.
    ///
    /// If the directory does not exist it is created. If `root` exists but is not a
    /// directory, an [`StoreError::Io`] is returned.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let metadata = fs::metadata(&root)?;
        if !metadata.is_dir() {
            return Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                format!("content store root {} is not a directory", root.display()),
            )));
        }
        Ok(Self { root })
    }

    /// Borrow the root directory path. Useful for diagnostics; callers should not derive
    /// file paths from this for direct manipulation.
    #[inline]
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compute the on-disk path for a given ref.
    fn path_for(&self, r: &ContentRef) -> PathBuf {
        self.root.join(r.to_hex())
    }
}

impl ContentStore for LocalFsContentStore {
    type Payload = Bytes;

    fn put(&self, bytes: &[u8]) -> Result<ContentRef, StoreError> {
        let r = ContentRef::of(bytes);
        let final_path = self.path_for(&r);

        // Idempotent: if the object already exists at the content-addressed name, the
        // dedup contract says the call is a no-op. We trust the name — the asymmetry rule
        // (D5) says re-hashing-on-write would be defensive against backend corruption, but
        // `content-store.md` §11 explicitly makes re-hash-on-read an opt-in audit mode.
        if final_path.exists() {
            return Ok(r);
        }

        // Write-to-temp in the same directory, then atomic rename. Temp lives in `root` so
        // the rename is filesystem-local.
        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.write_all(bytes)?;
        temp.as_file_mut().sync_all()?;
        // persist: atomic rename on Unix; on Windows it falls back to a non-atomic rename
        // but the same content-addressed naming makes a racing overwrite harmless (the
        // bytes are identical).
        match temp.persist(&final_path) {
            Ok(_) => Ok(r),
            Err(persist_err) => {
                // If another concurrent writer beat us to the same ref, persist may fail
                // (file already exists on some platforms). Re-check before surfacing the
                // error — concurrent identical puts are not a failure.
                if final_path.exists() {
                    Ok(r)
                } else {
                    Err(StoreError::Io(persist_err.error))
                }
            }
        }
    }

    fn get(&self, r: &ContentRef) -> Result<Self::Payload, NotFound> {
        let path = self.path_for(r);
        match fs::read(&path) {
            Ok(buf) => Ok(Bytes::from(buf)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(NotFound),
            Err(_) => Err(NotFound),
        }
    }

    fn delete(&self, r: &ContentRef) -> Result<(), StoreError> {
        let path = self.path_for(r);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(StoreError::Io(err)),
        }
    }

    fn list_refs<'a>(&'a self) -> Box<dyn Iterator<Item = ContentRef> + 'a> {
        let Ok(read_dir) = fs::read_dir(&self.root) else {
            return Box::new(std::iter::empty());
        };
        let iter = read_dir.filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            let s = name.to_str()?;
            decode_hex_hash(s).map(ContentRef::from_bytes)
        });
        Box::new(iter)
    }

    fn contains(&self, r: &ContentRef) -> bool {
        self.path_for(r).is_file()
    }
}

/// Decode a 64-character lowercase-hex string into the 32 raw hash bytes, returning `None`
/// for anything that doesn't match the expected shape (tempfiles, stray non-object files,
/// case variations).
fn decode_hex_hash(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = [0u8; 32];
    for (i, chunk) in bytes.chunks_exact(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

#[inline]
fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(10 + c - b'a'),
        _ => None,
    }
}
