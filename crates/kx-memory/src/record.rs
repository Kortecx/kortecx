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
    /// How many times this memory has been recalled — salience, from the off-digest
    /// `memory_decay` sidecar (0 if never accessed). Display / decay-policy only.
    pub access_count: u32,
    /// The unix-ms time this memory was last recalled (0 if never). Display only.
    pub last_accessed_ms: i64,
    /// `Some(ms)` if this memory has been decayed (soft-tombstoned) — reversible via
    /// [`crate::MemoryStore::restore`]; `None` if live. Off-digest; the `memories`
    /// row itself is NEVER deleted (that is the reversibility guarantee).
    pub tombstoned_ms: Option<i64>,
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

/// A read request into [`crate::MemoryStore::bundle`] — gather a set of memories for
/// the model to consolidate (distill into one durable semantic fact). A struct (not a
/// long arg list) so the seam stays readable and additive.
#[derive(Clone, Copy, Debug)]
pub struct BundleRequest<'a> {
    /// The isolation scope (server-derived caller principal).
    pub namespace: &'a str,
    /// Restrict to one kind (consolidation passes `Some(Episodic)`); `None` = any.
    pub kind: Option<MemoryKind>,
    /// `Some(vec)` ⇒ semantically-scoped (the entries most similar to this query,
    /// re-ranked by cosine); `None` ⇒ pure recency (newest-first).
    pub query_vec: Option<&'a [f32]>,
    /// Inclusive `created_ms` window `lo..=hi` (a wall-clock READ filter, off every
    /// hash); `None` = unbounded.
    pub window_ms: Option<(i64, i64)>,
    /// The embed-model fingerprint the query vector was produced under (the
    /// vector-space guard); empty ⇒ opt out. Ignored on the recency path.
    pub embed_fingerprint: &'a str,
    /// The maximum number of memories to bundle.
    pub limit: usize,
}

/// The eviction policy for [`crate::MemoryStore::decay`]. A memory is a candidate iff
/// it is BOTH older than `ttl_ms` AND under-recalled (`access_count < min_access`) — a
/// salient (frequently-recalled) old fact is protected. `dry_run` previews without
/// tombstoning anything.
#[derive(Clone, Copy, Debug)]
pub struct DecayPolicy {
    /// Age threshold in ms — a memory older than this is TTL-eligible.
    pub ttl_ms: i64,
    /// Salience floor — a memory recalled at least this many times is protected.
    pub min_access: u32,
    /// `true` ⇒ compute + return candidates but tombstone NOTHING (preview).
    pub dry_run: bool,
}

/// One memory that a [`DecayPolicy`] matched (would-be or actual eviction). The
/// `memories` row is never deleted — this is a reversible soft-tombstone.
#[derive(Clone, Debug)]
pub struct DecayCandidate {
    /// The content-addressed id (the citation key).
    pub memory_id: ContentRef,
    /// The remembered payload bytes (so a preview can show the snippet).
    pub content: Vec<u8>,
    /// Semantic vs episodic.
    pub kind: MemoryKind,
    /// The unix-ms write time (display only).
    pub created_ms: i64,
    /// How many times it was recalled (salience).
    pub access_count: u32,
    /// The unix-ms time it was last recalled (0 if never).
    pub last_accessed_ms: i64,
}

/// The outcome of a [`crate::MemoryStore::decay`] sweep.
#[derive(Clone, Debug)]
pub struct DecayReport {
    /// The memories the policy matched (previewed or evicted).
    pub candidates: Vec<DecayCandidate>,
    /// The number actually tombstoned this call (0 on a dry run).
    pub swept: usize,
    /// The number of live memories that survived the sweep (not matched).
    pub kept: usize,
    /// Whether this was a preview (`dry_run`).
    pub dry_run: bool,
}

/// A namespace's memory statistics ([`crate::MemoryStore::stats`]). Live counts
/// exclude tombstoned rows; `tombstoned` counts the soft-evicted (restorable) ones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryStats {
    /// Live (non-tombstoned) memory count.
    pub total: usize,
    /// Live semantic memories.
    pub semantic: usize,
    /// Live episodic memories.
    pub episodic: usize,
    /// Tombstoned (decayed, restorable) memories.
    pub tombstoned: usize,
    /// The namespace's fixed embedding dimension (0 if empty).
    pub dim: u32,
    /// The embed-model fingerprint the namespace was indexed under.
    pub fingerprint: String,
    /// The oldest live memory's `created_ms` (0 if empty).
    pub oldest_ms: i64,
    /// The newest live memory's `created_ms` (0 if empty).
    pub newest_ms: i64,
}

/// Cosine similarity of two equal-length finite vectors (the re-rank used by the
/// semantically-scoped [`crate::MemoryStore::bundle`] path). Returns `0.0` for a
/// length mismatch or a zero-magnitude vector — a safe, order-neutral default.
pub(crate) fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 0.0 {
        0.0
    } else {
        dot / denom
    }
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
