//! The durable multi-tier MEMORY read/write seam (RC5a — `StoreMemory` /
//! `ListMemories` / `RecallMemory` / `ForgetMemory`).
//!
//! Spoken entirely in gateway-core's OWN wire vocabulary (`&[u8]` / `Vec<f32>` /
//! `String`) — no `kx-memory` type crosses the seam, so gateway-core gains NO memory
//! crate dependency and stays off the writer wall. The host (`kx-gateway`, behind the
//! opt-in `hnsw` feature) implements [`MemoryView`] over `kx-memory`'s durable
//! `memory.db` store + the embedder.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** [`MemoryHitEntry::score`] is DISPLAY-ONLY — it never enters a
//!   committed fact or a `MoteId`; only the ordered memory-ref SET is the durable
//!   recall result. A `None` seam ⇒ the four RPCs return `unimplemented` (old-gateway
//!   / memory-disabled forward-compat degrade).
//! - **Server-derived namespace.** The `namespace` is the caller's ISOLATION scope,
//!   derived from the authenticated principal at the service layer — a client NEVER
//!   scopes into another principal's memories (it reaches the seam pre-scoped).
//! - **Content-addressed id.** The host derives each memory's id from its content
//!   (content-addressed, idempotent); a client never supplies an id on write.
//! - **Embedding is pluggable.** A store/recall may carry a client-computed vector
//!   (the FFI-free path) or rely on a server embedder (the `inference` path); the
//!   seam carries the optional vector and lets the host decide.

use kx_proto::proto;
use tonic::Status;

/// The kind of a memory — gateway-core's own vocab (maps to/from the wire enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MemoryKindTag {
    /// A durable fact the agent learned.
    #[default]
    Semantic,
    /// An event/observation from a run.
    Episodic,
}

impl MemoryKindTag {
    /// The stable wire tag returned in a [`proto::MemorySummary`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Episodic => "episodic",
        }
    }
}

/// Derive a memory NAMESPACE from the server-resolved caller principal (RC5a).
/// Scoped so a client can NEVER reach another principal's memories (verdict #5): the
/// caller-supplied `sub` is an OPTIONAL sub-bucket ALWAYS prefixed by the principal
/// (empty ⇒ the caller's default bucket). Both components are sanitized so the `::`
/// separator can never be smuggled to escape the `mem::<principal>` prefix.
///
/// SHARED so the RPC handlers AND the in-run `recall@1` / `remember@1` capabilities
/// (bound at construction to the serve's primary principal) derive the SAME namespace
/// — alignment by construction (an agent's `remember` and the operator's `kx memory
/// list` see one bucket on a single-node serve; per-run multi-party = Cloud, D129).
#[must_use]
pub fn memory_namespace(principal: &str, sub: &str) -> String {
    let p = sanitize_ns_component(principal);
    if sub.is_empty() {
        format!("mem::{p}")
    } else {
        format!("mem::{p}::{}", sanitize_ns_component(sub))
    }
}

/// Sanitize one namespace component to the memory-store allowlist (`[A-Za-z0-9._-]`,
/// bounded) — every other char (including `:`) becomes `-`, so a component can never
/// inject the `::` separator that scopes the principal prefix. Empty ⇒ `"default"`.
fn sanitize_ns_component(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// Map the wire enum discriminant to gateway-core's [`MemoryKindTag`]. Unknown /
/// `UNSPECIFIED` ⇒ `Semantic` (the default), never an error.
pub(crate) fn memory_kind_from_proto(v: i32) -> MemoryKindTag {
    match proto::MemoryKind::try_from(v) {
        Ok(proto::MemoryKind::Episodic) => MemoryKindTag::Episodic,
        _ => MemoryKindTag::Semantic,
    }
}

/// A write into the memory store. `embedding == None` requires a server embedder
/// (the `inference` path); `Some` is the FFI-free client-vector path. Borrows from
/// the request so the handler does not copy the payload before the host dedups it.
#[derive(Clone, Copy, Debug)]
pub struct MemoryWrite<'a> {
    /// The caller's server-derived isolation scope.
    pub namespace: &'a str,
    /// The payload to remember (the host content-addresses this for the id).
    pub content: &'a [u8],
    /// The client-computed vector, or `None` to ask the host to embed `content`.
    pub embedding: Option<&'a [f32]>,
    /// Semantic vs episodic.
    pub kind: MemoryKindTag,
    /// The run writing this memory (all-zero for an operator/SDK write).
    pub instance_id: [u8; 16],
}

/// The outcome of a [`MemoryView::store`] call.
#[derive(Clone, Copy, Debug)]
pub struct StoreMemoryOutcome {
    /// The content-addressed id of the (new or existing) memory.
    pub memory_id: [u8; 32],
    /// `true` if a NEW row was written; `false` on a content-addressed dedup hit.
    pub inserted: bool,
    /// The namespace's embedding dimension.
    pub dim: u32,
}

/// One memory in a [`MemoryView::list`] enumeration (the episodic-log view).
#[derive(Clone, Debug)]
pub struct MemoryEntry {
    /// The 32-byte content-addressed id (the citation key).
    pub memory_id: [u8; 32],
    /// The remembered payload bytes (so a UI can show the snippet).
    pub content: Vec<u8>,
    /// Semantic vs episodic.
    pub kind: MemoryKindTag,
    /// The run (`instance_id`) that wrote this memory (all-zero = operator/SDK write).
    pub instance_id: [u8; 16],
    /// The unix-ms write time (display only; off every hash).
    pub created_ms: i64,
    /// The memory vector's embedding dimension.
    pub dim: u32,
    /// RC5b: recall count (salience; off-digest sidecar). Display only.
    pub access_count: u32,
    /// RC5b: last recall time, unix-ms (0 = never). Display only.
    pub last_accessed_ms: i64,
    /// RC5b: `Some(ms)` if decayed (soft-tombstoned, restorable); `None` if live.
    pub tombstoned_ms: Option<i64>,
}

/// One memory a decay policy matched ([`MemoryView::decay`]). The store row is never
/// deleted — a soft, reversible tombstone.
#[derive(Clone, Debug)]
pub struct DecayCandidateEntry {
    /// The 32-byte content-addressed id.
    pub memory_id: [u8; 32],
    /// The payload bytes (preview snippet).
    pub content: Vec<u8>,
    /// Semantic vs episodic.
    pub kind: MemoryKindTag,
    /// The unix-ms write time.
    pub created_ms: i64,
    /// Recall count (salience).
    pub access_count: u32,
    /// Last recall time, unix-ms (0 = never).
    pub last_accessed_ms: i64,
}

/// The outcome of a [`MemoryView::decay`] sweep.
#[derive(Clone, Debug)]
pub struct DecayReportEntry {
    /// The memories the policy matched (previewed or evicted).
    pub candidates: Vec<DecayCandidateEntry>,
    /// The number actually tombstoned this call (0 on a dry run).
    pub evicted: usize,
    /// The number of live memories that survived the sweep.
    pub kept: usize,
    /// Whether this was a preview (`dry_run`).
    pub dry_run: bool,
}

/// Namespace statistics ([`MemoryView::stats`]).
#[derive(Clone, Debug, Default)]
pub struct MemoryStatsEntry {
    /// Live (non-tombstoned) memory count.
    pub total: u32,
    /// Live semantic memories.
    pub semantic: u32,
    /// Live episodic memories.
    pub episodic: u32,
    /// Tombstoned (restorable) memories.
    pub tombstoned: u32,
    /// The namespace embedding dimension (0 = empty).
    pub dim: u32,
    /// The embed-model fingerprint the namespace was indexed under.
    pub embed_fingerprint: String,
    /// The oldest live memory's `created_ms` (0 = empty).
    pub oldest_ms: i64,
    /// The newest live memory's `created_ms` (0 = empty).
    pub newest_ms: i64,
}

/// One recall hit. `score` is DISPLAY-ONLY (SN-8).
#[derive(Clone, Debug)]
pub struct MemoryHitEntry {
    /// The 32-byte content-addressed id of the recalled memory (the citation key).
    pub memory_id: [u8; 32],
    /// The remembered payload bytes.
    pub content: Vec<u8>,
    /// The similarity score — DISPLAY-ONLY; NEVER an identity input.
    pub score: f32,
}

/// A failure from the [`MemoryView`] seam, mapped to honest gRPC codes by the
/// service handler.
#[derive(Debug)]
pub enum MemoryError {
    /// The namespace does not exist ⇒ `not_found` (reserved for operations that
    /// require an existing namespace; recall soft-returns empty instead).
    NotFound,
    /// A vector's length disagrees with the namespace's fixed dimension ⇒
    /// `invalid_argument`.
    DimMismatch(String),
    /// A server-embed was requested (a vector-less store / `query_text`) but no
    /// embedder is wired ⇒ `failed_precondition`.
    EmbedderUnavailable,
    /// The live embedder's fingerprint disagrees with the one the namespace was
    /// indexed under (a different embed model) — querying would compare incompatible
    /// vector spaces ⇒ `failed_precondition`. Forget + re-remember to rebuild.
    StaleIndex(String),
    /// A malformed request (empty/oversize content, a bad namespace, non-UTF-8 text
    /// for a server-embed) ⇒ `invalid_argument`.
    InvalidArgument(String),
    /// A backend failure (store / persist / poisoned lock) ⇒ `internal`.
    Internal(String),
}

/// The memory read/write seam. The host implements it over `kx-memory` +
/// the embedder (behind the `hnsw` feature). A `None` seam on the service ⇒
/// the four memory RPCs return `unimplemented`.
pub trait MemoryView: Send + Sync {
    /// Remember `w.content` in `w.namespace`. A write carrying a vector uses the
    /// client-vector path; a vector-less write needs an embedder.
    ///
    /// # Errors
    /// [`MemoryError`] on a bad namespace/content, a dim mismatch, a missing embedder,
    /// a stale index, or a backend failure.
    fn store(&self, w: MemoryWrite<'_>) -> Result<StoreMemoryOutcome, MemoryError>;

    /// Recall the top-`k` memories in `namespace` most similar to the query.
    /// `query_embedding` (`Some`) is the client-vector path; `None` falls back to
    /// embedding `query_text` (needs an embedder). An unknown/empty namespace yields
    /// an empty result, never an error.
    ///
    /// # Errors
    /// [`MemoryError`] on a missing embedder, a dim mismatch, a stale index, or a
    /// backend failure.
    fn recall(
        &self,
        namespace: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
    ) -> Result<Vec<MemoryHitEntry>, MemoryError>;

    /// The episodic log of `namespace`, newest-first, at most `limit` rows,
    /// optionally scoped to the run `instance_filter`. `include_tombstoned` = `false`
    /// hides decayed memories (default view); `true` surfaces them (the decayed view).
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
        include_tombstoned: bool,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Gather a set of (live) memories for the model to consolidate — the newest
    /// (`query_text == None`) or most-similar-to-a-query (`Some`, host-embedded) slice
    /// of `namespace`, optionally restricted by kind + a `created_ms` window. The
    /// returned entries carry NO score (SN-8 by return type).
    ///
    /// # Errors
    /// [`MemoryError`] on a bad namespace / missing embedder / dim mismatch / stale
    /// index / backend failure.
    fn bundle(
        &self,
        namespace: &str,
        query_text: Option<&str>,
        kind_filter: Option<MemoryKindTag>,
        window_ms: Option<(i64, i64)>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Preview (`dry_run`) or apply a TTL + salience decay sweep over `namespace`.
    /// Evictions are reversible soft-tombstones (the store row is never deleted).
    ///
    /// # Errors
    /// [`MemoryError`] on a bad namespace / backend failure.
    fn decay(
        &self,
        namespace: &str,
        ttl_ms: i64,
        min_access: u32,
        dry_run: bool,
    ) -> Result<DecayReportEntry, MemoryError>;

    /// Namespace statistics — live counts by kind, tombstoned count, dim, fingerprint,
    /// and the live age range.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn stats(&self, namespace: &str) -> Result<MemoryStatsEntry, MemoryError>;

    /// Un-decay (restore) a soft-tombstoned memory. Returns `true` if a tombstone was
    /// cleared.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn restore(&self, namespace: &str, memory_id: &[u8; 32]) -> Result<bool, MemoryError>;

    /// Erase a memory from `namespace` (a HARD delete, not reversible — unlike decay).
    /// Returns `true` if a row was removed.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    fn forget(&self, namespace: &str, memory_id: &[u8; 32]) -> Result<bool, MemoryError>;
}

/// Map a [`MemoryError`] to its honest gRPC [`Status`].
pub(crate) fn memory_status(err: MemoryError) -> Status {
    match err {
        MemoryError::NotFound => Status::not_found("memory namespace not found"),
        MemoryError::DimMismatch(detail) | MemoryError::InvalidArgument(detail) => {
            Status::invalid_argument(detail)
        }
        MemoryError::EmbedderUnavailable => Status::failed_precondition(
            "no embedding model wired: provide a vector client-side, or run \
             `kx serve --features inference,hnsw` with a model",
        ),
        MemoryError::StaleIndex(detail) => Status::failed_precondition(detail),
        MemoryError::Internal(detail) => Status::internal(detail),
    }
}

/// Map a gateway-core memory entry into the wire type.
pub(crate) fn memory_summary_to_proto(e: MemoryEntry) -> proto::MemorySummary {
    proto::MemorySummary {
        memory_id: e.memory_id.to_vec(),
        content: e.content,
        kind: e.kind.as_str().to_string(),
        instance_id: e.instance_id.to_vec(),
        created_ms: e.created_ms,
        dim: e.dim,
        access_count: e.access_count,
        last_accessed_ms: e.last_accessed_ms,
        // 0 = live on the wire; Some(ms) = decayed (unix-ms is always > 0).
        tombstoned_ms: e.tombstoned_ms.unwrap_or(0),
    }
}

/// Map a gateway-core recall hit into the wire type.
pub(crate) fn memory_hit_to_proto(h: MemoryHitEntry) -> proto::MemoryHit {
    proto::MemoryHit {
        memory_id: h.memory_id.to_vec(),
        content: h.content,
        score: h.score,
    }
}

/// Map a gateway-core decay report into the wire type. `now_ms` derives each
/// candidate's `age_days` at sweep time (display only).
pub(crate) fn decay_report_to_proto(
    r: DecayReportEntry,
    now_ms: i64,
) -> proto::DecayMemoryResponse {
    let would_evict = u32::try_from(r.candidates.len()).unwrap_or(u32::MAX);
    let candidates = r
        .candidates
        .into_iter()
        .map(|c| proto::DecayCandidate {
            memory_id: c.memory_id.to_vec(),
            content: c.content,
            kind: c.kind.as_str().to_string(),
            created_ms: c.created_ms,
            access_count: c.access_count,
            last_accessed_ms: c.last_accessed_ms,
            age_days: u32::try_from(now_ms.saturating_sub(c.created_ms).max(0) / 86_400_000)
                .unwrap_or(u32::MAX),
        })
        .collect();
    proto::DecayMemoryResponse {
        candidates,
        would_evict,
        evicted: u32::try_from(r.evicted).unwrap_or(u32::MAX),
        kept: u32::try_from(r.kept).unwrap_or(u32::MAX),
        dry_run: r.dry_run,
    }
}

/// Map gateway-core namespace stats into the wire type.
pub(crate) fn memory_stats_to_proto(
    s: MemoryStatsEntry,
    namespace: String,
) -> proto::MemoryStatsResponse {
    proto::MemoryStatsResponse {
        total: s.total,
        semantic: s.semantic,
        episodic: s.episodic,
        tombstoned: s.tombstoned,
        dim: s.dim,
        embed_fingerprint: s.embed_fingerprint,
        oldest_ms: s.oldest_ms,
        newest_ms: s.newest_ms,
        namespace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn memory_status_maps_each_error_to_its_honest_code() {
        assert_eq!(memory_status(MemoryError::NotFound).code(), Code::NotFound);
        assert_eq!(
            memory_status(MemoryError::DimMismatch("d".into())).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            memory_status(MemoryError::InvalidArgument("a".into())).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            memory_status(MemoryError::EmbedderUnavailable).code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            memory_status(MemoryError::StaleIndex("stale".into())).code(),
            Code::FailedPrecondition
        );
        assert_eq!(
            memory_status(MemoryError::Internal("i".into())).code(),
            Code::Internal
        );
    }

    #[test]
    fn memory_namespace_is_principal_prefixed_and_unescapable() {
        // A default (empty sub) bucket is the caller's principal bucket.
        assert_eq!(memory_namespace("alice", ""), "mem::alice");
        // A sub-bucket stays UNDER the principal.
        assert_eq!(memory_namespace("alice", "work"), "mem::alice::work");
        // A client can NEVER inject `::` to escape into another principal: the `:`
        // in a malicious sub is sanitized to `-`, so it stays under `mem::alice::`.
        assert_eq!(
            memory_namespace("alice", "::bob"),
            "mem::alice::--bob",
            "a smuggled separator is neutralized — no cross-principal escape"
        );
        // Two principals never collide.
        assert_ne!(memory_namespace("alice", ""), memory_namespace("bob", ""));
    }

    #[test]
    fn memory_kind_from_proto_maps_known_and_defaults_unknown() {
        assert_eq!(
            memory_kind_from_proto(proto::MemoryKind::Episodic as i32),
            MemoryKindTag::Episodic
        );
        assert_eq!(
            memory_kind_from_proto(proto::MemoryKind::Semantic as i32),
            MemoryKindTag::Semantic
        );
        // UNSPECIFIED and any unknown discriminant ⇒ the default (Semantic).
        assert_eq!(memory_kind_from_proto(0), MemoryKindTag::Semantic);
        assert_eq!(memory_kind_from_proto(99), MemoryKindTag::Semantic);
    }
}
