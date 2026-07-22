// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! Operator-facing persistence: open an index from a cache file (rebuilding the
//! HNSW graph from records) and dump it back.
//!
//! Path validation rejects `..` traversal; the cache is a rebuildable projection,
//! so an absent file degrades to an empty index and a malformed file degrades to
//! a graceful error (the caller rebuilds from the journal) — never a panic.

use std::path::{Component, Path};

use kx_dataset::RetrievalIndex;

use crate::error::HnswError;
use crate::index::{HnswParams, HnswRetrievalIndex};
use crate::persist::{decode_records, encode_records};

/// Reject any path containing a parent-dir (`..`) component. Defence-in-depth:
/// the cache path is operator-configured, not request-derived, but the backend
/// refuses to follow a traversal regardless.
fn reject_traversal(path: &Path) -> Result<(), HnswError> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(HnswError::PathTraversal);
    }
    Ok(())
}

/// Open an index from `path` with default parameters. An absent file yields an
/// empty index (first run); a malformed file is a `Corrupt` error (the caller
/// rebuilds from the journal).
pub fn open(path: &Path) -> Result<HnswRetrievalIndex, HnswError> {
    open_with_params(path, HnswParams::default())
}

/// Open an index from `path` with explicit parameters. The graph is rebuilt by
/// re-inserting the cached records; the sizing hint is widened to the record
/// count for graph quality.
pub fn open_with_params(path: &Path, params: HnswParams) -> Result<HnswRetrievalIndex, HnswError> {
    reject_traversal(path)?;
    if !path.exists() {
        return Ok(HnswRetrievalIndex::with_params(params));
    }
    let bytes = std::fs::read(path)?;
    let (_dim, records) = decode_records(&bytes)?;
    let mut build = params;
    build.capacity_hint = records.len().max(params.capacity_hint).max(1);
    let mut index = HnswRetrievalIndex::with_params(build);
    for (id, vector) in records {
        index.insert(id, vector);
    }
    Ok(index)
}

/// Persist `index` to `path` as a rebuildable cache (records only; the graph is
/// rebuilt on open). Overwrites any existing file.
pub fn dump(index: &HnswRetrievalIndex, path: &Path) -> Result<(), HnswError> {
    reject_traversal(path)?;
    let (dim, ids, vectors) = index.snapshot();
    let bytes = encode_records(dim, ids, vectors);
    std::fs::write(path, bytes)?;
    Ok(())
}
