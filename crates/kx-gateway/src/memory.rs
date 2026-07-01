// SPDX-License-Identifier: Apache-2.0
//! [`HostMemoryView`] — the serve-side durable-MEMORY view (RC5a).
//!
//! A thin adapter that bridges the embedding-aware gateway-core [`MemoryView`] seam
//! to `kx-memory`'s vector-only [`kx_memory::SqliteMemoryStore`]: it resolves a text
//! payload/query to a vector (a client-supplied vector wins; else the server embedder
//! — reusing the SAME [`crate::datasets::HostEmbedder`] as the RAG path), stamps the
//! embed-model fingerprint, then delegates the durable store/recall/list/forget.
//!
//! Off the journal/digest — `memory.db` is a rebuildable sidecar (the datasets.db
//! posture, no journal schema bump). Namespace-scoped per caller principal (verdict
//! #5); the store's per-namespace similarity index makes isolation structural.

use std::path::Path;

use kx_gateway_core::{
    DecayCandidateEntry, DecayReportEntry, MemoryEntry, MemoryError, MemoryHitEntry, MemoryKindTag,
    MemoryStatsEntry, MemoryView, MemoryWrite, StoreMemoryOutcome,
};
use kx_memory::{MemoryError as StoreError, MemoryStore};

use crate::error::GatewayError;

/// The serve-side durable-MEMORY view over `kx-memory`'s `memory.db` store + the
/// server embedder (the `serve-engine` path). Behind the opt-in `hnsw` feature, like
/// the RAG `HostDatasetView`.
pub(crate) struct HostMemoryView {
    store: kx_memory::SqliteMemoryStore,
    #[cfg(feature = "serve-engine")]
    embedder: Option<crate::datasets::HostEmbedder>,
}

impl HostMemoryView {
    /// Open (or create) `memory.db` under `dir`, rebuilding the per-namespace indices
    /// from the durable rows before returning.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] if the store cannot be opened.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        let store = kx_memory::SqliteMemoryStore::open(dir)
            .map_err(|e| GatewayError::Catalog(format!("memory store: {e}")))?;
        Ok(Self {
            store,
            #[cfg(feature = "serve-engine")]
            embedder: None,
        })
    }

    /// Attach a server embedder (the `serve-engine` path), enabling text-only store
    /// and `query_text` recall.
    #[cfg(feature = "serve-engine")]
    #[must_use]
    pub(crate) fn with_embedder(mut self, embedder: crate::datasets::HostEmbedder) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Resolve a text payload/query to `(vector, embed_fingerprint)`. A client-supplied
    /// vector wins (the FFI-free path, no fingerprint guard); else the server embedder
    /// embeds `text` and stamps its fingerprint; else no embedder ⇒ `EmbedderUnavailable`.
    // Without `serve-engine` there is no embedder, so `text`/`self` are unused (the
    // client-vector path is the only one) — the FFI-free `hnsw`-alone build.
    #[cfg_attr(
        not(feature = "serve-engine"),
        allow(unused_variables, clippy::unused_self)
    )]
    fn resolve_vector(
        &self,
        provided: Option<&[f32]>,
        text: &str,
    ) -> Result<(Vec<f32>, String), MemoryError> {
        if let Some(v) = provided {
            return Ok((v.to_vec(), String::new()));
        }
        #[cfg(feature = "serve-engine")]
        if let Some(e) = self.embedder.as_ref() {
            let v = e
                .embed(text)
                .map_err(|err| MemoryError::Internal(format!("embedding: {err:?}")))?;
            return Ok((v, format!("{}:{}", e.model_id_string(), e.pooling_tag())));
        }
        Err(MemoryError::EmbedderUnavailable)
    }
}

/// Map a `kx-memory` store error to the gateway-core seam error (1:1).
fn map_store_err(e: StoreError) -> MemoryError {
    match e {
        StoreError::NotFound => MemoryError::NotFound,
        StoreError::DimMismatch(d) => MemoryError::DimMismatch(d),
        StoreError::StaleIndex(d) => MemoryError::StaleIndex(d),
        StoreError::InvalidArgument(d) => MemoryError::InvalidArgument(d),
        StoreError::Internal(d) => MemoryError::Internal(d),
    }
}

fn to_store_kind(k: MemoryKindTag) -> kx_memory::MemoryKind {
    match k {
        MemoryKindTag::Semantic => kx_memory::MemoryKind::Semantic,
        MemoryKindTag::Episodic => kx_memory::MemoryKind::Episodic,
    }
}

fn from_store_kind(k: kx_memory::MemoryKind) -> MemoryKindTag {
    match k {
        kx_memory::MemoryKind::Semantic => MemoryKindTag::Semantic,
        kx_memory::MemoryKind::Episodic => MemoryKindTag::Episodic,
    }
}

/// Map a durable `kx-memory` record into the gateway-core wire entry (shared by
/// `list` + `bundle`).
fn record_to_entry(r: kx_memory::MemoryRecord) -> MemoryEntry {
    MemoryEntry {
        memory_id: *r.memory_id.as_bytes(),
        content: r.content,
        kind: from_store_kind(r.kind),
        instance_id: r.instance_id,
        created_ms: r.created_ms,
        dim: r.dim,
        access_count: r.access_count,
        last_accessed_ms: r.last_accessed_ms,
        tombstoned_ms: r.tombstoned_ms,
    }
}

impl HostMemoryView {
    /// Apply a decay sweep across EVERY namespace (the operator auto-sweep on open).
    /// Best-effort; returns the total number of memories tombstoned.
    ///
    /// # Errors
    /// [`MemoryError::Internal`] on a backend failure.
    pub(crate) fn sweep_all(&self, ttl_ms: i64, min_access: u32) -> Result<usize, MemoryError> {
        self.store
            .decay_all(kx_memory::DecayPolicy {
                ttl_ms,
                min_access,
                dry_run: false,
            })
            .map_err(map_store_err)
    }
}

impl MemoryView for HostMemoryView {
    fn store(&self, w: MemoryWrite<'_>) -> Result<StoreMemoryOutcome, MemoryError> {
        // Server-embed needs valid UTF-8 text; a non-UTF-8 payload must supply a
        // client vector (else it is un-embeddable) — honest invalid_argument.
        let (vector, fp) = if w.embedding.is_some() {
            self.resolve_vector(w.embedding, "")?
        } else {
            let text = std::str::from_utf8(w.content).map_err(|_| {
                MemoryError::InvalidArgument(
                    "non-UTF-8 memory content requires a client-supplied vector".to_string(),
                )
            })?;
            self.resolve_vector(None, text)?
        };
        let out = self
            .store
            .store(kx_memory::StoreRequest {
                namespace: w.namespace,
                content: w.content,
                vector: &vector,
                kind: to_store_kind(w.kind),
                instance_id: w.instance_id,
                created_ms: kx_memory::now_ms(),
                embed_fingerprint: &fp,
            })
            .map_err(map_store_err)?;
        Ok(StoreMemoryOutcome {
            memory_id: *out.memory_id.as_bytes(),
            inserted: out.inserted,
            dim: out.dim,
        })
    }

    fn recall(
        &self,
        namespace: &str,
        query_embedding: Option<&[f32]>,
        query_text: &str,
        k: usize,
    ) -> Result<Vec<MemoryHitEntry>, MemoryError> {
        let (vector, fp) = self.resolve_vector(query_embedding, query_text)?;
        let hits = self
            .store
            .recall(namespace, &vector, k, &fp)
            .map_err(map_store_err)?;
        Ok(hits
            .into_iter()
            .map(|h| MemoryHitEntry {
                memory_id: *h.memory_id.as_bytes(),
                content: h.content,
                score: h.score,
            })
            .collect())
    }

    fn list(
        &self,
        namespace: &str,
        instance_filter: Option<[u8; 16]>,
        limit: usize,
        include_tombstoned: bool,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let rows = self
            .store
            .list(namespace, instance_filter, limit, include_tombstoned)
            .map_err(map_store_err)?;
        Ok(rows.into_iter().map(record_to_entry).collect())
    }

    fn bundle(
        &self,
        namespace: &str,
        query_text: Option<&str>,
        kind_filter: Option<MemoryKindTag>,
        window_ms: Option<(i64, i64)>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let kind = kind_filter.map(to_store_kind);
        // A non-empty query focuses the bundle semantically (host-embedded); else recency.
        let query = query_text.filter(|t| !t.is_empty());
        let rows = if let Some(text) = query {
            let (vector, fp) = self.resolve_vector(None, text)?;
            self.store
                .bundle(kx_memory::BundleRequest {
                    namespace,
                    kind,
                    query_vec: Some(&vector),
                    window_ms,
                    embed_fingerprint: &fp,
                    limit,
                })
                .map_err(map_store_err)?
        } else {
            self.store
                .bundle(kx_memory::BundleRequest {
                    namespace,
                    kind,
                    query_vec: None,
                    window_ms,
                    embed_fingerprint: "",
                    limit,
                })
                .map_err(map_store_err)?
        };
        Ok(rows.into_iter().map(record_to_entry).collect())
    }

    fn decay(
        &self,
        namespace: &str,
        ttl_ms: i64,
        min_access: u32,
        dry_run: bool,
    ) -> Result<DecayReportEntry, MemoryError> {
        let report = self
            .store
            .decay(
                namespace,
                kx_memory::DecayPolicy {
                    ttl_ms,
                    min_access,
                    dry_run,
                },
            )
            .map_err(map_store_err)?;
        Ok(DecayReportEntry {
            candidates: report
                .candidates
                .into_iter()
                .map(|c| DecayCandidateEntry {
                    memory_id: *c.memory_id.as_bytes(),
                    content: c.content,
                    kind: from_store_kind(c.kind),
                    created_ms: c.created_ms,
                    access_count: c.access_count,
                    last_accessed_ms: c.last_accessed_ms,
                })
                .collect(),
            evicted: report.swept,
            kept: report.kept,
            dry_run: report.dry_run,
        })
    }

    fn stats(&self, namespace: &str) -> Result<MemoryStatsEntry, MemoryError> {
        let s = self.store.stats(namespace).map_err(map_store_err)?;
        Ok(MemoryStatsEntry {
            total: u32::try_from(s.total).unwrap_or(u32::MAX),
            semantic: u32::try_from(s.semantic).unwrap_or(u32::MAX),
            episodic: u32::try_from(s.episodic).unwrap_or(u32::MAX),
            tombstoned: u32::try_from(s.tombstoned).unwrap_or(u32::MAX),
            dim: s.dim,
            embed_fingerprint: s.fingerprint,
            oldest_ms: s.oldest_ms,
            newest_ms: s.newest_ms,
        })
    }

    fn restore(&self, namespace: &str, memory_id: &[u8; 32]) -> Result<bool, MemoryError> {
        let mid = kx_content::ContentRef::from_bytes(*memory_id);
        self.store.restore(namespace, &mid).map_err(map_store_err)
    }

    fn forget(&self, namespace: &str, memory_id: &[u8; 32]) -> Result<bool, MemoryError> {
        let mid = kx_content::ContentRef::from_bytes(*memory_id);
        self.store.forget(namespace, &mid).map_err(map_store_err)
    }
}
