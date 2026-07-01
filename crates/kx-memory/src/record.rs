// SPDX-License-Identifier: Apache-2.0
//! The memory record vocabulary — [`MemoryKind`], [`MemoryRecord`], [`MemoryHit`],
//! the write [`StoreRequest`]/[`StoreOutcome`], the content-addressed [`memory_id`],
//! and the shared validation + encoding helpers.

use std::time::{SystemTime, UNIX_EPOCH};

use kx_content::ContentRef;
use serde::{Deserialize, Serialize};

/// The maximum payload size of a single memory (bytes). A remembered fact is a
/// short note, not a document — larger content belongs in a dataset. Bounded so an
/// agent cannot smuggle an unbounded blob into the memory store.
pub const MAX_CONTENT_LEN: usize = 8 * 1024;

/// The maximum namespace length (a SQLite key, not a filename).
pub const MAX_NAMESPACE_LEN: usize = 128;

/// What kind of memory this is — a classification carried alongside the payload; it
/// does NOT change how the memory is indexed or recalled (both kinds are embedded +
/// recallable + listable). Consolidation/decay (RC5b) reason over the kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// A durable fact the agent learned ("the deadline is March 3rd").
    #[default]
    Semantic,
    /// An event/observation from a run ("the user asked about pricing").
    Episodic,
}

impl MemoryKind {
    /// The stable wire tag (`"semantic"` / `"episodic"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Episodic => "episodic",
        }
    }

    /// The integer discriminant persisted in the durable row (stable; append-only).
    #[must_use]
    pub const fn as_i64(self) -> i64 {
        match self {
            Self::Semantic => 0,
            Self::Episodic => 1,
        }
    }

    /// Recover a [`MemoryKind`] from its persisted discriminant. An unknown value
    /// (a forward-written row read by an older binary would never happen here, but a
    /// corrupt db might) degrades to the safe default [`MemoryKind::Semantic`].
    #[must_use]
    pub const fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Episodic,
            _ => Self::Semantic,
        }
    }
}

/// One stored memory (the episodic-log view). `content` is the payload the agent
/// remembered; `instance_id` is the run that wrote it (all-zero for an operator/SDK
/// write); `created_ms` is display-only (off every hash).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRecord {
    /// The content-addressed id (== [`memory_id`] of `content`) — the citation key.
    pub memory_id: ContentRef,
    /// The isolation scope (the server-derived caller principal).
    pub namespace: String,
    /// The remembered payload bytes.
    pub content: Vec<u8>,
    /// Semantic vs episodic (metadata; does not change indexing).
    pub kind: MemoryKind,
    /// The run (`instance_id`) that wrote this memory; all-zero = operator/SDK write.
    pub instance_id: [u8; 16],
    /// The unix-ms write time (display only; off every hash).
    pub created_ms: i64,
    /// The embedding dimension of this memory's vector (0 if unknown).
    pub dim: u32,
}

/// One recall result — a content-addressed memory + its similarity score.
/// `score` is DISPLAY-ONLY (SN-8): it never enters a committed fact or a `MoteId`.
#[derive(Clone, Debug)]
pub struct MemoryHit {
    /// The recalled memory's content-addressed id (the citation key).
    pub memory_id: ContentRef,
    /// The remembered payload bytes (for the agent to read / a UI to show).
    pub content: Vec<u8>,
    /// The similarity score — DISPLAY-ONLY; NEVER an identity input.
    pub score: f32,
}

/// A write request into [`crate::MemoryStore::store`]. A struct (not a long arg
/// list) so the seam stays readable and additive.
#[derive(Clone, Copy, Debug)]
pub struct StoreRequest<'a> {
    /// The isolation scope (server-derived caller principal).
    pub namespace: &'a str,
    /// The payload to remember (content-addressed for the id + dedup).
    pub content: &'a [u8],
    /// The embedding of `content` (the caller embeds; the store never does).
    pub vector: &'a [f32],
    /// Semantic vs episodic.
    pub kind: MemoryKind,
    /// The run writing this memory (all-zero for an operator/SDK write).
    pub instance_id: [u8; 16],
    /// The unix-ms write time (display only).
    pub created_ms: i64,
    /// The embed-model fingerprint the vector was produced under (the cross-model
    /// vector-space guard). Empty ⇒ the caller opts out of the guard.
    pub embed_fingerprint: &'a str,
}

/// The outcome of a [`crate::MemoryStore::store`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreOutcome {
    /// The content-addressed id of the (new or existing) memory.
    pub memory_id: ContentRef,
    /// `true` if a NEW row was written; `false` if a content-addressed dedup hit
    /// (the same payload was already remembered in this namespace).
    pub inserted: bool,
    /// The namespace's embedding dimension after this store.
    pub dim: u32,
}

/// The content-addressed identity of a memory — the [`ContentRef`] of its payload.
/// Idempotent on the bytes (the same fact remembered twice dedups to one row, keyed
/// `(namespace, memory_id)`), so an exactly-once pre-commit re-dispatch is a no-op.
#[must_use]
pub fn memory_id(content: &[u8]) -> ContentRef {
    ContentRef::of(content)
}

/// Canonical little-endian f32 encoding of a vector — byte-identical to the RAG
/// `encode_vector_le`, so a memory vector round-trips reproducibly in the durable row.
pub(crate) fn encode_vector_le(vector: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode the canonical little-endian f32 form. A trailing partial chunk (a corrupt
/// row) is dropped rather than panicking.
pub(crate) fn decode_vector_le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// `true` iff every component is finite (no `NaN`/±inf). An untrusted vector must
/// pass this before it touches the cosine index — a `NaN`/inf component would poison
/// the similarity ordering (and is meaningless as an embedding).
pub(crate) fn all_finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// Wall-clock unix-ms (display only, off every hash). A pre-epoch clock ⇒ 0.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Validate a request-supplied namespace: non-empty, bounded, an ASCII
/// `[A-Za-z0-9._:-]` allowlist (`:` allows the `mem::<principal>` convention), never
/// a bare dot run. A SQLite key, so this is hygiene.
pub(crate) fn validate_namespace(ns: &str) -> Result<(), crate::MemoryError> {
    if ns.is_empty() || ns.len() > MAX_NAMESPACE_LEN {
        return Err(crate::MemoryError::InvalidArgument(format!(
            "namespace must be 1..={MAX_NAMESPACE_LEN} chars"
        )));
    }
    if ns == "." || ns == ".." {
        return Err(crate::MemoryError::InvalidArgument(
            "invalid namespace".to_string(),
        ));
    }
    if !ns
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
    {
        return Err(crate::MemoryError::InvalidArgument(
            "namespace allows [A-Za-z0-9._:-] only".to_string(),
        ));
    }
    Ok(())
}

/// Validate the payload of a `store`: non-empty and within [`MAX_CONTENT_LEN`].
pub(crate) fn validate_content(content: &[u8]) -> Result<(), crate::MemoryError> {
    if content.is_empty() {
        return Err(crate::MemoryError::InvalidArgument(
            "memory content must be non-empty".to_string(),
        ));
    }
    if content.len() > MAX_CONTENT_LEN {
        return Err(crate::MemoryError::InvalidArgument(format!(
            "memory content must be <= {MAX_CONTENT_LEN} bytes (got {})",
            content.len()
        )));
    }
    Ok(())
}
