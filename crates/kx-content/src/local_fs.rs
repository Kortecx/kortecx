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

// ---------------------------------------------------------------------------
// D39 Test A — store atomicity seam (test-only)
//
// Production `put` writes a NamedTempFile then atomic-renames it into place. The
// guarantee this seam exists to PROVE is: a worker that dies AFTER sync_all but
// BEFORE the atomic rename leaves no observable object at the canonical ref. The
// existing `obligation_3` test asserts that a non-persisted tempfile is not
// observable (via NamedTempFile's Drop auto-cleanup); that proves the happy
// path, not the crash path. This seam simulates the crash path by allowing the
// test to short-circuit BEFORE persist AND intentionally LEAK the temp file (so
// Drop does not clean up — modeling a process that died mid-flight).
//
// The seam is `pub(crate)` and gated `#[cfg(test)]` — invisible outside test
// builds and outside the crate. Production callers see only the trait surface.
// ---------------------------------------------------------------------------

#[cfg(test)]
impl LocalFsContentStore {
    /// Test-only seam: run `put`'s prefix (compute ref, write temp, sync_all),
    /// then call `interrupt`. If `interrupt` returns `true`, the function
    /// returns BEFORE calling `persist` AND keeps the temp file on disk via
    /// `TempPath::keep()` (suppressing Drop) — modeling a worker that died
    /// after sync_all but before the atomic rename. If `interrupt` returns
    /// `false`, `persist` runs and the call completes normally.
    ///
    /// Returns `(ContentRef, persisted)` where `persisted` is `true` iff the
    /// atomic rename happened. **Contract verified**: when `persisted == false`,
    /// the canonical ref is invisible through every read method (`get`,
    /// `contains`, `list_refs`) — the temp file is on disk but at a name that
    /// is not the canonical hash, and `list_refs` filters it because the temp
    /// filename does not decode as 64-character lowercase hex.
    pub(crate) fn put_with_interrupt_hook<F>(
        &self,
        bytes: &[u8],
        interrupt: F,
    ) -> Result<(ContentRef, bool), StoreError>
    where
        F: FnOnce() -> bool,
    {
        let r = ContentRef::of(bytes);
        let final_path = self.path_for(&r);

        if final_path.exists() {
            return Ok((r, true));
        }

        let mut temp = tempfile::NamedTempFile::new_in(&self.root)?;
        temp.write_all(bytes)?;
        temp.as_file_mut().sync_all()?;

        if interrupt() {
            // Suppress Drop: the temp file persists on disk at its random name,
            // modeling an orphan from a crashed worker.
            let _path = temp
                .into_temp_path()
                .keep()
                .map_err(|e| StoreError::Io(std::io::Error::other(e)))?;
            return Ok((r, false));
        }

        match temp.persist(&final_path) {
            Ok(_) => Ok((r, true)),
            Err(persist_err) => {
                if final_path.exists() {
                    Ok((r, true))
                } else {
                    Err(StoreError::Io(persist_err.error))
                }
            }
        }
    }
}

#[cfg(test)]
mod atomicity_seam_tests {
    use super::{ContentRef, ContentStore, LocalFsContentStore, NotFound};
    use tempfile::TempDir;

    /// D39 Test A — primary obligation.
    ///
    /// After an interrupt between sync_all and persist:
    /// (a) `get(ref) → NotFound`
    /// (b) `contains(ref) == false`
    /// (c) `list_refs()` does NOT include the canonical ref
    /// (d) the orphan temp file IS still on disk (proves the seam genuinely
    ///     skipped Drop — not relying on `NamedTempFile` cleanup to pass)
    #[test]
    fn put_interrupted_between_sync_and_persist_leaves_no_observable_canonical_ref() {
        let tmp = TempDir::new().expect("tempdir");
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let payload = b"interrupted-payload-d39-test-a";

        let (r, persisted) = store
            .put_with_interrupt_hook(payload, || true)
            .expect("seam returns Ok even when interrupt fires");

        assert!(!persisted, "interrupt must prevent persist");

        // (a) get() returns NotFound at the canonical ref.
        assert!(
            matches!(store.get(&r), Err(NotFound)),
            "get(canonical_ref) MUST be NotFound after interrupt"
        );

        // (b) contains() returns false.
        assert!(
            !store.contains(&r),
            "contains(canonical_ref) MUST be false after interrupt"
        );

        // (c) list_refs() does NOT include the canonical ref. The orphan
        //     tempfile is present on disk but at a non-hex filename, so
        //     decode_hex_hash filters it out — list_refs is empty.
        let listed: Vec<ContentRef> = store.list_refs().collect();
        assert!(
            !listed.contains(&r),
            "list_refs() MUST NOT include canonical_ref after interrupt"
        );
        assert!(
            listed.is_empty(),
            "list_refs() MUST be empty (orphan temp must not parse as a canonical hex name)"
        );

        // (d) the orphan temp file IS still on disk — proves the seam suppressed
        //     Drop (modeling a real crash), not just relied on auto-cleanup to
        //     pass the test.
        let dir_entries: Vec<_> = std::fs::read_dir(tmp.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(
            dir_entries.len(),
            1,
            "exactly one orphan file MUST remain on disk after interrupt \
             (proves Drop was suppressed; if 0, the seam isn't modeling a real crash)"
        );

        // The orphan's filename must NOT be the canonical hex (would have meant
        // persist ran somehow).
        let orphan_name = dir_entries[0].file_name();
        let orphan_str = orphan_name.to_string_lossy();
        assert_ne!(
            orphan_str.as_ref(),
            r.to_hex(),
            "orphan filename MUST NOT be the canonical hex (would mean persist ran)"
        );
    }

    /// The seam is a controlled switch: when `interrupt` returns `false`, the
    /// path is identical to production `put` and the canonical ref is observable.
    /// Guards against the seam itself silently breaking the happy path.
    #[test]
    fn put_with_interrupt_hook_proceeds_normally_when_hook_returns_false() {
        let tmp = TempDir::new().expect("tempdir");
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let payload = b"normal-payload-no-interrupt";

        let (r, persisted) = store
            .put_with_interrupt_hook(payload, || false)
            .expect("ok");

        assert!(persisted, "no interrupt → persist must run");
        assert!(store.contains(&r));
        assert_eq!(&*store.get(&r).expect("present"), payload);
    }

    /// Recovery: after an interrupted attempt, a subsequent normal `put` with
    /// identical bytes MUST succeed and place the canonical file. Proves that
    /// an interrupt does not permanently poison the store for that ref — the
    /// orphan temp does not block recovery (uniqueness is per-attempt because
    /// `NamedTempFile` generates a fresh random name each call).
    #[test]
    fn put_after_interrupt_can_recover_via_subsequent_normal_put() {
        let tmp = TempDir::new().expect("tempdir");
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let payload = b"recovery-payload-d39-test-a";

        let (r1, persisted1) = store
            .put_with_interrupt_hook(payload, || true)
            .expect("first attempt seam runs");
        assert!(!persisted1, "first attempt was interrupted");
        assert!(!store.contains(&r1), "canonical ref absent after interrupt");

        // Recovery: normal put with same bytes. Same canonical ref derives;
        // the temp orphan from attempt 1 is still on disk but at a different
        // random name, so it does not block this put.
        let r2 = store.put(payload).expect("recovery put succeeds");
        assert_eq!(
            r1, r2,
            "ref derivation is bytes-pure; recovery returns same ref"
        );
        assert!(store.contains(&r2), "canonical ref present after recovery");
        assert_eq!(
            &*store.get(&r2).expect("present"),
            payload,
            "recovered bytes match"
        );
    }

    /// Idempotence under repeated interrupts: two interrupted attempts in a
    /// row, then a successful normal put. Proves multiple orphans do not
    /// accumulate at the canonical name (they accumulate as separate temp
    /// files; canonical name is occupied exactly once after the eventual
    /// successful put).
    #[test]
    fn repeated_interrupts_then_recovery_yields_exactly_one_canonical_object() {
        let tmp = TempDir::new().expect("tempdir");
        let store = LocalFsContentStore::open(tmp.path()).expect("open");
        let payload = b"repeated-interrupt-payload";

        let (r, p1) = store.put_with_interrupt_hook(payload, || true).expect("ok");
        assert!(!p1);
        let (_, p2) = store.put_with_interrupt_hook(payload, || true).expect("ok");
        assert!(!p2);
        let r3 = store.put(payload).expect("recovery");
        assert_eq!(r, r3);

        // The canonical file exists exactly once.
        assert!(store.contains(&r));
        let canonical = store.list_refs().filter(|x| *x == r).count();
        assert_eq!(
            canonical, 1,
            "exactly one object at canonical_ref after recovery"
        );
    }
}
