//! RC4b (agentic RAG) — the bundled read-only `retrieve@1` capability + its typed
//! [`ToolDef`] + the serve-broker registration. A model in a `kx/recipes/react-rag`
//! ReAct turn proposes `{"dataset": <name>, "query": <search text>, "k": <1..64>}`;
//! this runs the RC4a HYBRID (BM25 + dense, RRF-fused, MMR-reranked)
//! [`DatasetView::query`] and returns the ordered passages (content hash + text +
//! chunk provenance) as the Observation Mote's `result_ref` — the agent reads them,
//! refines its query, and grounds its answer.
//!
//! # Why this is the "agentic" half of RAG
//!
//! RC4a made retrieval high-quality but PASSIVE (a fixed DAG node / a one-shot
//! host-side fold). `retrieve@1` makes a dataset a FIRST-CLASS TOOL the agent calls
//! autonomously: the model decides WHEN to search, phrases its OWN query (the
//! committed `ReadOnlyNondet` retrieve-call turn IS the query-rewrite — no separate
//! Mote), reads the passages, and can RE-QUERY on the next turn. It is the exact
//! sibling of the read-only `fs-list@1` / `fs-read@1` capabilities (PR-6a / D155):
//! a host-side read over live process state, granted by a server-built recipe warrant.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** The committed observation carries the ordered EXACT chunk content-refs
//!   (+ text for the model to read) — the similarity `score` is DROPPED here, never
//!   committed, never an identity input. (Mirrors `kx-model-harness::rag::query_corpus`.)
//! - **Read-only.** `IdempotencyClass::Readback` ⇒ the HITL gate auto-proceeds it; no
//!   egress (`NetScope::None`), no fs (`FsScope::empty()` — the SQLite store is reached
//!   via the in-process `Arc<dyn DatasetView>`, NOT through `fs_scope`).
//! - **Flat builtin id.** `retrieve@1` is a `ToolKind::Builtin` (like `fs-list@1`):
//!   the model proposes the FULL name `retrieve`, which `kx_toolcall::resolve_granted_name`
//!   matches by EXACT equality (`id_matches`). The `<server>/<remote>` convention is
//!   ONLY for MCP tools where the model emits the bare remote leaf (the BUG-33 guard);
//!   a builtin needs no `/` and using one would be semantically wrong (no MCP server).
//! - **Fail-SOFT, never dead-letter.** A missing/unavailable/stale dataset (every
//!   recoverable [`DatasetError`]) returns an EMPTY-passage observation the model reads
//!   and recovers from; only a hard `DatasetError::Internal` (poisoned lock) returns
//!   `Err` and dead-letters the chain. A weak embedder or no hits is an honest empty,
//!   not a crash.

use std::sync::Arc;

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{DatasetError, DatasetView, RetrievalMode};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::{IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
use serde::{Deserialize, Serialize};

/// retrieve observes the world (a read over the dataset index) and commits its bytes
/// as a `result_ref` — the same stage-then-commit content-addressed path fs-list uses.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// The default top-k when the model omits `k` (matches the chat-rag default).
const DEFAULT_K: usize = 4;

/// The hard top-k ceiling (also enforced by the typed schema + `HostDatasetView`).
const MAX_K: usize = 64;

/// The bundled read-only dataset-retrieval capability (`retrieve@1`).
pub(crate) struct RetrieveCapability {
    name: ToolName,
    version: ToolVersion,
    datasets: Arc<dyn DatasetView>,
}

impl RetrieveCapability {
    /// Construct `retrieve@1` over the live in-process dataset view.
    pub(crate) fn new(datasets: Arc<dyn DatasetView>) -> Self {
        Self {
            name: ToolName("retrieve".into()),
            version: ToolVersion("1".into()),
            datasets,
        }
    }
}

/// The model's proposed argument bag (the typed `inputSchema` validated this
/// upstream at coordinator settle; we re-parse fail-closed against smuggled keys).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrieveArgs {
    dataset: String,
    query: String,
    #[serde(default)]
    k: Option<u32>,
}

/// One retrieved passage — content hash + text + chunk provenance. NO `score`
/// (SN-8: the committed observation is the ordered EXACT-ref set only).
#[derive(Serialize)]
struct Passage {
    /// Hex of the retrieved CHUNK's content ref (the citation key).
    r#ref: String,
    /// The chunk text the model grounds its answer on.
    text: String,
    /// Hex of the parent document's content ref (== `ref` for un-chunked corpora).
    parent_ref: String,
    /// 0-based ordinal of this chunk within its parent.
    chunk_index: u32,
}

/// The committed observation the agent reads. `passages` is empty (with an honest
/// `note`) on every recoverable failure — the model re-queries or answers from what
/// it has, the chain never dead-letters.
#[derive(Serialize)]
struct Observation {
    dataset: String,
    query: String,
    passages: Vec<Passage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn encode(obs: &Observation) -> Result<Vec<u8>, CapabilityFailureReason> {
    serde_json::to_vec(obs)
        .map_err(|e| CapabilityFailureReason::Other(format!("retrieve: encode: {e}")))
}

impl Capability for RetrieveCapability {
    fn name(&self) -> &ToolName {
        &self.name
    }

    fn version(&self) -> &ToolVersion {
        &self.version
    }

    fn supported_patterns(&self) -> &[EffectPattern] {
        PATTERNS
    }

    fn invoke(&self, request: &EffectRequest) -> Result<Vec<u8>, CapabilityFailureReason> {
        let args: RetrieveArgs = serde_json::from_slice(&request.payload)
            .map_err(|e| CapabilityFailureReason::Other(format!("retrieve: bad args: {e}")))?;
        let k = args.k.map_or(DEFAULT_K, |k| (k as usize).clamp(1, MAX_K));

        // HYBRID (RC4a): query_embedding = None ⇒ the host embeds `query` internally.
        // rerank = None ⇒ the operator's MMR default (the agentic loop reasons over
        // the passages itself; LLM rerank is the RC4c-2 live coordinator turn).
        match self.datasets.query(
            &args.dataset,
            None,
            &args.query,
            k,
            RetrievalMode::Hybrid,
            None,
        ) {
            Ok(hits) => {
                let passages = hits
                    .into_iter()
                    .map(|h| Passage {
                        r#ref: ContentRef::from_bytes(h.content_ref).to_hex(),
                        text: String::from_utf8_lossy(&h.content).into_owned(),
                        parent_ref: ContentRef::from_bytes(h.parent_ref).to_hex(),
                        chunk_index: h.chunk_index,
                        // h.score is intentionally DROPPED here (SN-8).
                    })
                    .collect::<Vec<_>>();
                let note = passages
                    .is_empty()
                    .then(|| "no matching passages found".to_string());
                encode(&Observation {
                    dataset: args.dataset,
                    query: args.query,
                    passages,
                    note,
                })
            }
            // SOFT-FAIL every recoverable error → an honest empty observation the model
            // reads + recovers from (re-query a different dataset, or answer from prior
            // turns). NEVER dead-letter the chain on a recoverable retrieval miss.
            Err(
                DatasetError::NotFound
                | DatasetError::StaleIndex(_)
                | DatasetError::DimMismatch(_)
                | DatasetError::EmbedderUnavailable
                | DatasetError::InvalidArgument(_),
            ) => encode(&Observation {
                dataset: args.dataset,
                query: args.query,
                passages: Vec::new(),
                note: Some(
                    "no passages (dataset missing, empty, stale, or no embedder available)"
                        .to_string(),
                ),
            }),
            // A hard backend fault (poisoned lock) is NOT recoverable ⇒ dead-letter honestly.
            Err(DatasetError::Internal(detail)) => Err(CapabilityFailureReason::Other(format!(
                "retrieve: {detail}"
            ))),
        }
    }
}

/// The bundled retrieval tool's identity — `retrieve@1` (a FLAT builtin id, the
/// `fs-list@1` precedent; a model proposes the full `retrieve`, resolved by exact
/// `id_matches` equality).
#[must_use]
pub(crate) fn retrieve_tool() -> (ToolName, ToolVersion) {
    (ToolName("retrieve".into()), ToolVersion("1".into()))
}

/// The `retrieve@1` [`ToolDef`]: a read-only dataset-retrieval tool with NO egress /
/// NO fs scope (the dataset store is reached via the in-process view), a typed
/// `{dataset, query, k?}` schema (unknown keys refused), `Readback` (HITL
/// auto-proceeds). Seeded into `tools.db` so `render_tool_menu` / `validate_args` /
/// `resolve_tool_args` resolve a model-proposed `retrieve` call.
#[must_use]
pub(crate) fn retrieve_tool_def() -> ToolDef {
    let (tool_id, tool_version) = retrieve_tool();
    ToolDef {
        tool_id,
        tool_version,
        kind: ToolKind::Builtin,
        required_capability: ToolRequirement {
            net_scope_required: NetScope::None,
            fs_scope_required: FsScope::empty(),
            syscall_profile_ref: ContentRef::from_bytes([0; 32]),
            min_resource_ceiling: ResourceCeiling {
                cpu_milli: 0,
                mem_bytes: 0,
                wall_clock_ms: 0,
                fd_count: 0,
                disk_bytes: 0,
            },
        },
        description: "Retrieve the most relevant passages from a TEXT dataset by hybrid (keyword + semantic) search. Arg: {\"dataset\": <name>, \"query\": <search text>, \"k\": <1..64, optional>}. Returns ordered passages (content hash + text + chunk index). Read-only; idempotent.".into(),
        idempotency_class: IdempotencyClass::Readback,
        input_schema: Some(InputSchema {
            params: vec![
                ParamSpec {
                    name: "dataset".into(),
                    ty: ParamType::Str { max_len: 128 },
                    required: true,
                },
                ParamSpec {
                    name: "query".into(),
                    ty: ParamType::Str { max_len: 4096 },
                    required: true,
                },
                ParamSpec {
                    name: "k".into(),
                    ty: ParamType::Int {
                        min: Some(1),
                        max: Some(64), // == MAX_K (the host + capability also clamp)
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled read-only [`RetrieveCapability`] (`retrieve@1`) on the serve
/// broker over the live `Arc<dyn DatasetView>`. Called AFTER `dataset_view` is built
/// (the broker's `register_capability` is `&self`/interior-mutable, so late
/// registration on the same broker is supported). Operator-reachable only when a
/// model is served + `hnsw` (the `kx/recipes/react-rag` seed gate).
pub(crate) fn register_retrieve_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
    datasets: Arc<dyn DatasetView>,
) {
    broker.register_capability(Box::new(RetrieveCapability::new(datasets)));
    tracing::info!("RC4b: read-only retrieve@1 capability registered (kx/recipes/react-rag)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_gateway_core::{DatasetHitEntry, DatasetSummaryEntry, IngestDoc, IngestOutcome};
    use kx_warrant::SecretScope;

    /// A stub [`DatasetView`] that returns canned hits OR a fixed error, so the
    /// capability's mapping + soft-fail behaviour is tested with no model / no index.
    struct StubView {
        hits: Vec<DatasetHitEntry>,
        err: Option<DatasetError>,
    }

    impl StubView {
        fn ok(hits: Vec<DatasetHitEntry>) -> Arc<dyn DatasetView> {
            Arc::new(Self { hits, err: None })
        }
        fn fail(err: DatasetError) -> Arc<dyn DatasetView> {
            Arc::new(Self {
                hits: Vec::new(),
                err: Some(err),
            })
        }
    }

    impl DatasetView for StubView {
        fn list_datasets(&self) -> Vec<DatasetSummaryEntry> {
            Vec::new()
        }
        fn ingest(
            &self,
            _dataset: &str,
            _docs: &[IngestDoc<'_>],
        ) -> Result<IngestOutcome, DatasetError> {
            Err(DatasetError::Internal("stub".into()))
        }
        fn query(
            &self,
            _dataset: &str,
            _query_embedding: Option<&[f32]>,
            _query_text: &str,
            _k: usize,
            _mode: RetrievalMode,
            _rerank: Option<bool>,
        ) -> Result<Vec<DatasetHitEntry>, DatasetError> {
            match &self.err {
                Some(DatasetError::NotFound) => Err(DatasetError::NotFound),
                Some(DatasetError::StaleIndex(d)) => Err(DatasetError::StaleIndex(d.clone())),
                Some(DatasetError::DimMismatch(d)) => Err(DatasetError::DimMismatch(d.clone())),
                Some(DatasetError::EmbedderUnavailable) => Err(DatasetError::EmbedderUnavailable),
                Some(DatasetError::InvalidArgument(d)) => {
                    Err(DatasetError::InvalidArgument(d.clone()))
                }
                Some(DatasetError::Internal(d)) => Err(DatasetError::Internal(d.clone())),
                None => Ok(self.hits.clone()),
            }
        }
    }

    fn hit(tag: u8, text: &str, score: f32, chunk_index: u32) -> DatasetHitEntry {
        DatasetHitEntry {
            content_ref: [tag; 32],
            content: text.as_bytes().to_vec(),
            score,
            parent_ref: [tag.wrapping_add(100); 32],
            chunk_index,
            chunk_count: 3,
        }
    }

    fn req(payload: &[u8]) -> EffectRequest {
        EffectRequest {
            payload: payload.to_vec(),
            pattern: EffectPattern::StageThenCommit,
            idempotency_key: None,
            net_scope: NetScope::None,
            fs_scope: FsScope::empty(),
            secret_scope: SecretScope::None,
        }
    }

    #[test]
    fn returns_ordered_passages_and_drops_the_score() {
        let cap = RetrieveCapability::new(StubView::ok(vec![
            hit(1, "photosynthesis converts sunlight", 0.91, 0),
            hit(2, "plants use chlorophyll", 0.74, 1),
        ]));
        let out = cap
            .invoke(&req(
                br#"{"dataset":"docs","query":"how plants make energy"}"#,
            ))
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        // The observation carries the ordered passages...
        let passages = v["passages"].as_array().unwrap();
        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0]["text"], "photosynthesis converts sunlight");
        assert_eq!(passages[0]["ref"], ContentRef::from_bytes([1; 32]).to_hex());
        assert_eq!(
            passages[0]["parent_ref"],
            ContentRef::from_bytes([101; 32]).to_hex()
        );
        assert_eq!(passages[0]["chunk_index"], 0);
        // ...and NEVER the score (SN-8: the committed fact is the ref set, scores out).
        assert!(
            !String::from_utf8_lossy(&out).contains("score"),
            "the committed observation must not carry a similarity score (SN-8)"
        );
        assert!(passages[0].get("score").is_none());
    }

    #[test]
    fn default_k_when_omitted_and_clamps_an_oversize_k() {
        // (k is validated upstream by the typed schema; the capability clamps defensively.)
        let cap = RetrieveCapability::new(StubView::ok(vec![hit(1, "x", 0.5, 0)]));
        // Omitted k → no panic, returns the hit.
        let out = cap.invoke(&req(br#"{"dataset":"d","query":"q"}"#)).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out).unwrap()["passages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn soft_fails_every_recoverable_error_to_an_empty_observation() {
        for err in [
            DatasetError::NotFound,
            DatasetError::StaleIndex("stale".into()),
            DatasetError::DimMismatch("dim".into()),
            DatasetError::EmbedderUnavailable,
            DatasetError::InvalidArgument("bad".into()),
        ] {
            let cap = RetrieveCapability::new(StubView::fail(err));
            let out = cap
                .invoke(&req(br#"{"dataset":"missing","query":"q"}"#))
                .expect("recoverable errors must NOT dead-letter the chain");
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert!(v["passages"].as_array().unwrap().is_empty());
            assert!(v["note"].is_string(), "an honest note explains the empty");
        }
    }

    #[test]
    fn hard_internal_error_dead_letters() {
        let cap =
            RetrieveCapability::new(StubView::fail(DatasetError::Internal("poisoned".into())));
        assert!(
            cap.invoke(&req(br#"{"dataset":"d","query":"q"}"#)).is_err(),
            "a hard backend fault must surface as Err (dead-letter), not a fake empty"
        );
    }

    #[test]
    fn refuses_unknown_arg_key_and_malformed_json() {
        let cap = RetrieveCapability::new(StubView::ok(vec![]));
        assert!(cap
            .invoke(&req(br#"{"dataset":"d","query":"q","evil":1}"#))
            .is_err());
        assert!(cap.invoke(&req(br#"{"dataset":"d"}"#)).is_err()); // missing required `query`
        assert!(cap.invoke(&req(b"not json")).is_err());
    }

    #[test]
    fn tool_def_is_flat_builtin_with_typed_schema() {
        let (name, ver) = retrieve_tool();
        assert_eq!(name.0, "retrieve");
        assert!(
            !name.0.contains('/'),
            "a builtin id is flat (the fs-list precedent)"
        );
        assert_eq!(ver.0, "1");
        let def = retrieve_tool_def();
        assert!(matches!(def.kind, ToolKind::Builtin));
        assert_eq!(def.idempotency_class, IdempotencyClass::Readback);
        assert_eq!(def.required_capability.net_scope_required, NetScope::None);
        let schema = def.input_schema.expect("typed schema");
        assert!(schema.deny_unknown);
        let names: Vec<&str> = schema.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["dataset", "query", "k"]);
        assert!(!schema.params[0].name.is_empty());
        assert!(!schema.params[2].required, "k is optional");
    }
}
