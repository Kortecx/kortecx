// SPDX-License-Identifier: Apache-2.0
//! RC5a (durable memory) — the bundled read-only `recall@1` capability + its typed
//! [`ToolDef`] + the serve-broker registration. A model in a `kx/recipes/react-memory`
//! ReAct turn proposes `{"query": <search text>, "k": <1..64>}`; this recalls the
//! most-similar memories in the run's namespace and returns them (content + citation
//! ref) as the Observation Mote's `result_ref` — the agent reads them and grounds its
//! answer on what it learned in a PRIOR run.
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** The committed observation carries the ordered EXACT memory content-refs
//!   (+ text for the model to read) — the similarity `score` is DROPPED, never
//!   committed, never an identity input. (Mirrors `retrieve@1` / `query_corpus`.)
//! - **Read-only.** `IdempotencyClass::Readback` ⇒ the HITL gate auto-proceeds it; no
//!   egress (`NetScope::None`), no fs (`FsScope::empty()` — the store is reached via the
//!   in-process `Arc<dyn MemoryView>`, NOT `fs_scope`).
//! - **Server-injected namespace.** The isolation scope is BOUND at registration to the
//!   serve's primary principal (verdict #5) — a model NEVER proposes it, so `recall`
//!   can never reach another principal's memories.
//! - **Fail-SOFT, never dead-letter.** Every recoverable memory error returns an
//!   EMPTY-memory observation the model reads + recovers from; only a hard
//!   `MemoryError::Internal` returns `Err` and dead-letters the chain.

use std::sync::Arc;

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{MemoryError, MemoryView};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::{IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
use serde::{Deserialize, Serialize};

/// recall observes the world (a read over the memory index) and commits its bytes as a
/// `result_ref` — the same stage-then-commit content-addressed path retrieve@1 uses.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// The default top-k when the model omits `k`.
const DEFAULT_K: usize = 5;

/// The hard top-k ceiling.
const MAX_K: usize = 64;

/// The bundled read-only memory-recall capability (`recall@1`), bound to the serve's
/// memory namespace (verdict #5 — the model never scopes it).
pub(crate) struct RecallCapability {
    name: ToolName,
    version: ToolVersion,
    memory: Arc<dyn MemoryView>,
    namespace: String,
}

impl RecallCapability {
    pub(crate) fn new(memory: Arc<dyn MemoryView>, namespace: String) -> Self {
        Self {
            name: ToolName("recall".into()),
            version: ToolVersion("1".into()),
            memory,
            namespace,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecallArgs {
    query: String,
    #[serde(default)]
    k: Option<u32>,
}

/// One recalled memory — content hash + text. NO `score` (SN-8).
#[derive(Serialize)]
struct Recalled {
    r#ref: String,
    text: String,
}

#[derive(Serialize)]
struct Observation {
    query: String,
    memories: Vec<Recalled>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn encode(obs: &Observation) -> Result<Vec<u8>, CapabilityFailureReason> {
    serde_json::to_vec(obs)
        .map_err(|e| CapabilityFailureReason::Other(format!("recall: encode: {e}")))
}

impl Capability for RecallCapability {
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
        let args: RecallArgs = serde_json::from_slice(&request.payload)
            .map_err(|e| CapabilityFailureReason::Other(format!("recall: bad args: {e}")))?;
        let k = args.k.map_or(DEFAULT_K, |k| (k as usize).clamp(1, MAX_K));
        // query_embedding = None ⇒ the host embeds `query` internally.
        match self.memory.recall(&self.namespace, None, &args.query, k) {
            Ok(hits) => {
                let memories = hits
                    .into_iter()
                    .map(|h| Recalled {
                        r#ref: ContentRef::from_bytes(h.memory_id).to_hex(),
                        text: String::from_utf8_lossy(&h.content).into_owned(),
                        // h.score is intentionally DROPPED here (SN-8).
                    })
                    .collect::<Vec<_>>();
                let note = memories
                    .is_empty()
                    .then(|| "no relevant memories found".to_string());
                encode(&Observation {
                    query: args.query,
                    memories,
                    note,
                })
            }
            // SOFT-FAIL every recoverable error → an honest empty observation the model
            // reads + recovers from. NEVER dead-letter the chain on a recoverable miss.
            Err(
                MemoryError::NotFound
                | MemoryError::DimMismatch(_)
                | MemoryError::EmbedderUnavailable
                | MemoryError::StaleIndex(_)
                | MemoryError::InvalidArgument(_),
            ) => encode(&Observation {
                query: args.query,
                memories: Vec::new(),
                note: Some("no memories (empty, stale, or no embedder available)".to_string()),
            }),
            // A hard backend fault is NOT recoverable ⇒ dead-letter honestly.
            Err(MemoryError::Internal(detail)) => {
                Err(CapabilityFailureReason::Other(format!("recall: {detail}")))
            }
        }
    }
}

/// The bundled recall tool's identity — `recall@1` (a FLAT builtin id, the retrieve@1
/// precedent; a model proposes the full `recall`, resolved by exact `id_matches`).
#[must_use]
pub(crate) fn recall_tool() -> (ToolName, ToolVersion) {
    (ToolName("recall".into()), ToolVersion("1".into()))
}

/// The `recall@1` [`ToolDef`]: a read-only memory-recall tool with NO egress / NO fs
/// scope (the store is reached via the in-process view), a typed `{query, k?}` schema
/// (unknown keys refused), `Readback` (HITL auto-proceeds).
#[must_use]
pub(crate) fn recall_tool_def() -> ToolDef {
    let (tool_id, tool_version) = recall_tool();
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
        description: "Recall the most relevant facts you REMEMBERED in earlier runs, by semantic search. Arg: {\"query\": <what to recall>, \"k\": <1..64, optional>}. Returns ordered memories (content hash + text). Read-only; idempotent. Use it to ground your answer on what you already learned.".into(),
        idempotency_class: IdempotencyClass::Readback,
        input_schema: Some(InputSchema {
            params: vec![
                ParamSpec {
                    name: "query".into(),
                    ty: ParamType::Str { max_len: 4096 },
                    required: true,
                },
                ParamSpec {
                    name: "k".into(),
                    ty: ParamType::Int {
                        min: Some(1),
                        max: Some(64), // == MAX_K
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled read-only [`RecallCapability`] (`recall@1`) on the serve broker
/// over the live `Arc<dyn MemoryView>`, bound to the serve's memory `namespace`.
pub(crate) fn register_recall_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
    memory: Arc<dyn MemoryView>,
    namespace: String,
) {
    broker.register_capability(Box::new(RecallCapability::new(memory, namespace)));
    tracing::info!("RC5a: read-only recall@1 capability registered (kx/recipes/react-memory)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_gateway_core::{MemoryEntry, MemoryHitEntry, MemoryWrite, StoreMemoryOutcome};
    use kx_warrant::SecretScope;

    /// A stub [`MemoryView`] returning canned hits OR a fixed error.
    struct StubView {
        hits: Vec<MemoryHitEntry>,
        err: Option<MemoryError>,
    }

    impl StubView {
        fn ok(hits: Vec<MemoryHitEntry>) -> Arc<dyn MemoryView> {
            Arc::new(Self { hits, err: None })
        }
        fn fail(err: MemoryError) -> Arc<dyn MemoryView> {
            Arc::new(Self {
                hits: Vec::new(),
                err: Some(err),
            })
        }
    }

    impl MemoryView for StubView {
        fn store(&self, _w: MemoryWrite<'_>) -> Result<StoreMemoryOutcome, MemoryError> {
            Err(MemoryError::Internal("stub".into()))
        }
        fn recall(
            &self,
            _namespace: &str,
            _qe: Option<&[f32]>,
            _query_text: &str,
            _k: usize,
        ) -> Result<Vec<MemoryHitEntry>, MemoryError> {
            match &self.err {
                Some(MemoryError::NotFound) => Err(MemoryError::NotFound),
                Some(MemoryError::DimMismatch(d)) => Err(MemoryError::DimMismatch(d.clone())),
                Some(MemoryError::EmbedderUnavailable) => Err(MemoryError::EmbedderUnavailable),
                Some(MemoryError::StaleIndex(d)) => Err(MemoryError::StaleIndex(d.clone())),
                Some(MemoryError::InvalidArgument(d)) => {
                    Err(MemoryError::InvalidArgument(d.clone()))
                }
                Some(MemoryError::Internal(d)) => Err(MemoryError::Internal(d.clone())),
                None => Ok(self.hits.clone()),
            }
        }
        fn list(
            &self,
            _namespace: &str,
            _instance_filter: Option<[u8; 16]>,
            _limit: usize,
        ) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(Vec::new())
        }
        fn forget(&self, _namespace: &str, _memory_id: &[u8; 32]) -> Result<bool, MemoryError> {
            Ok(false)
        }
    }

    fn hit(tag: u8, text: &str, score: f32) -> MemoryHitEntry {
        MemoryHitEntry {
            memory_id: [tag; 32],
            content: text.as_bytes().to_vec(),
            score,
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
    fn returns_ordered_memories_and_drops_the_score() {
        let cap = RecallCapability::new(
            StubView::ok(vec![
                hit(1, "the deadline is march 3rd", 0.91),
                hit(2, "the client prefers email", 0.74),
            ]),
            "mem::local-dev".into(),
        );
        let out = cap.invoke(&req(br#"{"query":"deadline"}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let mems = v["memories"].as_array().unwrap();
        assert_eq!(mems.len(), 2);
        assert_eq!(mems[0]["text"], "the deadline is march 3rd");
        assert_eq!(mems[0]["ref"], ContentRef::from_bytes([1; 32]).to_hex());
        assert!(
            !String::from_utf8_lossy(&out).contains("score"),
            "the committed observation must not carry a similarity score (SN-8)"
        );
    }

    #[test]
    fn soft_fails_every_recoverable_error_to_an_empty_observation() {
        for err in [
            MemoryError::NotFound,
            MemoryError::DimMismatch("d".into()),
            MemoryError::EmbedderUnavailable,
            MemoryError::StaleIndex("s".into()),
            MemoryError::InvalidArgument("a".into()),
        ] {
            let cap = RecallCapability::new(StubView::fail(err), "mem::x".into());
            let out = cap
                .invoke(&req(br#"{"query":"q"}"#))
                .expect("recoverable errors must NOT dead-letter the chain");
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert!(v["memories"].as_array().unwrap().is_empty());
            assert!(v["note"].is_string());
        }
    }

    #[test]
    fn hard_internal_error_dead_letters() {
        let cap = RecallCapability::new(
            StubView::fail(MemoryError::Internal("x".into())),
            "mem::x".into(),
        );
        assert!(cap.invoke(&req(br#"{"query":"q"}"#)).is_err());
    }

    #[test]
    fn refuses_unknown_arg_and_malformed_json() {
        let cap = RecallCapability::new(StubView::ok(vec![]), "mem::x".into());
        assert!(cap.invoke(&req(br#"{"query":"q","evil":1}"#)).is_err());
        assert!(cap.invoke(&req(br"{}")).is_err()); // missing required query
        assert!(cap.invoke(&req(b"not json")).is_err());
    }

    #[test]
    fn tool_def_is_flat_builtin_readback() {
        let (name, ver) = recall_tool();
        assert_eq!(name.0, "recall");
        assert!(!name.0.contains('/'));
        assert_eq!(ver.0, "1");
        let def = recall_tool_def();
        assert!(matches!(def.kind, ToolKind::Builtin));
        assert_eq!(def.idempotency_class, IdempotencyClass::Readback);
        assert_eq!(def.required_capability.net_scope_required, NetScope::None);
    }
}
