// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Operator-facing persistence: open an index from a cache file (rebuilding the
//! inverted index by re-tokenizing the records) and dump it back.
//!
//! Path validation rejects `..` traversal; the cache is a rebuildable projection,
//! so an absent file degrades to an empty index and a malformed file degrades to a
//! graceful error (the caller rebuilds from the journal/rows) — never a panic.

use std::path::{Component, Path};

use kx_dataset::LexicalIndex;

use crate::error::Bm25Error;
use crate::index::Bm25Index;
use crate::persist::{decode_records, encode_records};

/// Reject any path containing a parent-dir (`..`) component. Defence-in-depth: the
/// cache path is operator-configured, not request-derived, but the backend refuses
/// to follow a traversal regardless.
fn reject_traversal(path: &Path) -> Result<(), Bm25Error> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Bm25Error::PathTraversal);
    }
    Ok(())
}

/// Open an index from `path` with default parameters. An absent file yields an
/// empty index (first run); a malformed file is a `Corrupt` error (the caller
/// rebuilds from the journal/rows). The inverted index is rebuilt by re-tokenizing
/// the cached `(ref, text)` records with the CURRENT tokenizer.
pub fn open(path: &Path) -> Result<Bm25Index, Bm25Error> {
    reject_traversal(path)?;
    if !path.exists() {
        return Ok(Bm25Index::new());
    }
    let bytes = std::fs::read(path)?;
    let records = decode_records(&bytes)?;
    let mut index = Bm25Index::new();
    for (id, text) in records {
        index.insert(id, &text);
    }
    Ok(index)
}

/// Persist `index` to `path` as a rebuildable cache (records only; the inverted
/// index is rebuilt on open). Overwrites any existing file.
pub fn dump(index: &Bm25Index, path: &Path) -> Result<(), Bm25Error> {
    reject_traversal(path)?;
    let (ids, texts) = index.snapshot();
    let bytes = encode_records(ids, texts);
    std::fs::write(path, bytes)?;
    Ok(())
}
