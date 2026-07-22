// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! RC5b (durable memory — consolidation) — the bundled read-only `consolidate@1`
//! capability + its typed [`ToolDef`] + the serve-broker registration. In a
//! `kx/recipes/react-memory` turn a model proposes
//! `{"query"?, "k"?, "kind_filter"?, "window_hours"?}`; this BUNDLES the run's recent
//! (or most-similar) EPISODIC memories and returns them (content + citation ref) as the
//! Observation Mote's `result_ref`. The model reads the bundle, distills it, and calls
//! the existing `remember@1` to write ONE durable SEMANTIC fact — so consolidation is
//! a normal, replay-faithful react turn (no new journal fact, no journal bump).
//!
//! # Boundaries (load-bearing)
//!
//! - **SN-8.** The committed observation carries the ordered EXACT memory content-refs
//!   (+ text for the model to read) — no similarity `score`, no `created_ms` in the
//!   committed identity (both are display-only / off-hash). Mirrors `recall@1`.
//! - **Read-only.** `IdempotencyClass::Readback` ⇒ the HITL gate auto-proceeds it; no
//!   egress (`NetScope::None`), no fs (`FsScope::empty()` — the store is reached via the
//!   in-process `Arc<dyn MemoryView>`). The WRITE happens later, via `remember@1`.
//! - **Server-injected namespace.** Bound at registration to the serve's primary
//!   principal (verdict #5) — a model NEVER proposes it, so it can never bundle another
//!   principal's memories.
//! - **Fail-SOFT, never dead-letter.** Every recoverable memory error returns an
//!   EMPTY-bundle observation the model reads + recovers from; only a hard
//!   `MemoryError::Internal` returns `Err` and dead-letters the chain.

use std::sync::Arc;

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{MemoryError, MemoryKindTag, MemoryView};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::{IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
use serde::{Deserialize, Serialize};

/// consolidate observes the world (a read over the memory store) and commits its bytes
/// as a `result_ref` — the same stage-then-commit content-addressed path recall@1 uses.
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// The default number of memories to bundle when the model omits `k`.
const DEFAULT_K: usize = 16;

/// The hard bundle-size ceiling.
const MAX_K: usize = 64;

/// The maximum total payload bytes of a bundle (a defensive clamp so the consolidation
/// bundle can never overflow the model's input window; the tail is dropped with a note).
const MAX_BUNDLE_BYTES: usize = 24 * 1024;

/// The bundled read-only memory-consolidation capability (`consolidate@1`), bound to the
/// serve's memory namespace (verdict #5 — the model never scopes it).
pub(crate) struct ConsolidateCapability {
    name: ToolName,
    version: ToolVersion,
    memory: Arc<dyn MemoryView>,
    namespace: String,
}

impl ConsolidateCapability {
    pub(crate) fn new(memory: Arc<dyn MemoryView>, namespace: String) -> Self {
        Self {
            name: ToolName("consolidate".into()),
            version: ToolVersion("1".into()),
            memory,
            namespace,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConsolidateArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    k: Option<u32>,
    #[serde(default)]
    kind_filter: Option<String>,
    #[serde(default)]
    window_hours: Option<u32>,
}

/// One bundled memory — content hash + text. NO `score`, NO `created_ms` (SN-8: neither
/// is in the committed observation identity).
#[derive(Serialize)]
struct Bundled {
    r#ref: String,
    text: String,
}

#[derive(Serialize)]
struct Observation {
    #[serde(skip_serializing_if = "Option::is_none")]
    focus: Option<String>,
    entries: Vec<Bundled>,
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn encode(obs: &Observation) -> Result<Vec<u8>, CapabilityFailureReason> {
    serde_json::to_vec(obs)
        .map_err(|e| CapabilityFailureReason::Other(format!("consolidate: encode: {e}")))
}

/// Wall-clock unix-ms (for the optional `window_hours` READ filter — off every hash).
fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    )
    .unwrap_or(i64::MAX)
}

/// Resolve the `kind_filter` arg to a kind restriction. Consolidation defaults to
/// EPISODIC (distilling raw events into durable semantic knowledge); `"semantic"`
/// bundles semantic facts; `"any"`/`"all"` bundles every kind.
fn parse_kind_filter(s: Option<&str>) -> Option<MemoryKindTag> {
    match s.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("semantic") => Some(MemoryKindTag::Semantic),
        Some("any" | "all" | "") | None => {
            // "" ⇒ explicit "no kind restriction" is surprising for a default; keep the
            // default EPISODIC only when the field is ABSENT. An empty string ⇒ any.
            match s {
                None => Some(MemoryKindTag::Episodic),
                _ => None,
            }
        }
        // "episodic" and any unknown value ⇒ the safe default (episodic).
        _ => Some(MemoryKindTag::Episodic),
    }
}

impl Capability for ConsolidateCapability {
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
        let args: ConsolidateArgs = serde_json::from_slice(&request.payload)
            .map_err(|e| CapabilityFailureReason::Other(format!("consolidate: bad args: {e}")))?;
        let k = args.k.map_or(DEFAULT_K, |k| (k as usize).clamp(1, MAX_K));
        let kind = parse_kind_filter(args.kind_filter.as_deref());
        let window = args.window_hours.map(|h| {
            let now = now_ms();
            (
                now.saturating_sub(i64::from(h).saturating_mul(3_600_000)),
                now,
            )
        });
        let query = args.query.as_deref().filter(|q| !q.is_empty());
        match self.memory.bundle(&self.namespace, query, kind, window, k) {
            Ok(entries) => {
                // Bytes-budget clamp: keep the front (most relevant / recent), drop the
                // tail so the bundle can never overflow the model's input window.
                let mut used = 0usize;
                let mut truncated = false;
                let mut out: Vec<Bundled> = Vec::with_capacity(entries.len());
                for e in entries {
                    used += e.content.len();
                    if !out.is_empty() && used > MAX_BUNDLE_BYTES {
                        truncated = true;
                        break;
                    }
                    out.push(Bundled {
                        r#ref: ContentRef::from_bytes(e.memory_id).to_hex(),
                        text: String::from_utf8_lossy(&e.content).into_owned(),
                        // e.score does not exist (SN-8); e.created_ms is deliberately
                        // NOT committed (display-only, off every hash).
                    });
                }
                let note = if out.is_empty() {
                    Some("no episodic memories to consolidate".to_string())
                } else if truncated {
                    Some("bundle truncated to fit the input window".to_string())
                } else {
                    None
                };
                let count = out.len();
                encode(&Observation {
                    focus: query.map(str::to_string),
                    entries: out,
                    count,
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
                focus: query.map(str::to_string),
                entries: Vec::new(),
                count: 0,
                note: Some("no memories to consolidate (empty, stale, or no embedder)".to_string()),
            }),
            // A hard backend fault is NOT recoverable ⇒ dead-letter honestly.
            Err(MemoryError::Internal(detail)) => Err(CapabilityFailureReason::Other(format!(
                "consolidate: {detail}"
            ))),
        }
    }
}

/// The bundled consolidate tool's identity — `consolidate@1` (a FLAT builtin id, the
/// recall@1 precedent; a model proposes the full `consolidate`, resolved by exact
/// `id_matches`).
#[must_use]
pub(crate) fn consolidate_tool() -> (ToolName, ToolVersion) {
    (ToolName("consolidate".into()), ToolVersion("1".into()))
}

/// The `consolidate@1` [`ToolDef`]: a read-only memory-bundle tool with NO egress / NO
/// fs scope (the store is reached via the in-process view), a typed
/// `{query?, k?, kind_filter?, window_hours?}` schema (unknown keys refused), `Readback`
/// (HITL auto-proceeds).
#[must_use]
pub(crate) fn consolidate_tool_def() -> ToolDef {
    let (tool_id, tool_version) = consolidate_tool();
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
        description: "Bundle your recent EPISODIC memories so you can distill them into ONE durable fact. Arg: {\"query\": <optional focus>, \"k\": <1..64, optional>, \"kind_filter\": <\"episodic\"|\"semantic\"|\"any\", optional>, \"window_hours\": <optional recency window>}. Returns ordered entries (content hash + text). Read-only; idempotent. After reading, call `remember` with kind=\"semantic\" to save the distilled summary.".into(),
        idempotency_class: IdempotencyClass::Readback,
        input_schema: Some(InputSchema {
            params: vec![
                ParamSpec {
                    name: "query".into(),
                    ty: ParamType::Str { max_len: 4096 },
                    required: false,
                },
                ParamSpec {
                    name: "k".into(),
                    ty: ParamType::Int {
                        min: Some(1),
                        max: Some(64), // == MAX_K
                    },
                    required: false,
                },
                ParamSpec {
                    name: "kind_filter".into(),
                    ty: ParamType::Str { max_len: 16 },
                    required: false,
                },
                ParamSpec {
                    name: "window_hours".into(),
                    ty: ParamType::Int {
                        min: Some(1),
                        max: Some(8760), // one year
                    },
                    required: false,
                },
            ],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled read-only [`ConsolidateCapability`] (`consolidate@1`) on the
/// serve broker over the live `Arc<dyn MemoryView>`, bound to the serve's memory
/// `namespace`.
pub(crate) fn register_consolidate_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
    memory: Arc<dyn MemoryView>,
    namespace: String,
) {
    broker.register_capability(Box::new(ConsolidateCapability::new(memory, namespace)));
    tracing::info!("RC5b: read-only consolidate@1 capability registered (kx/recipes/react-memory)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_gateway_core::{
        DecayReportEntry, MemoryEntry, MemoryHitEntry, MemoryStatsEntry, MemoryWrite,
        StoreMemoryOutcome,
    };
    use kx_warrant::SecretScope;

    /// A stub [`MemoryView`] whose `bundle` returns canned entries OR a fixed error.
    struct StubView {
        entries: Vec<MemoryEntry>,
        err: Option<MemoryError>,
    }

    impl StubView {
        fn ok(entries: Vec<MemoryEntry>) -> Arc<dyn MemoryView> {
            Arc::new(Self { entries, err: None })
        }
        fn fail(err: MemoryError) -> Arc<dyn MemoryView> {
            Arc::new(Self {
                entries: Vec::new(),
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
            _n: &str,
            _q: Option<&[f32]>,
            _t: &str,
            _k: usize,
        ) -> Result<Vec<MemoryHitEntry>, MemoryError> {
            Ok(Vec::new())
        }
        fn list(
            &self,
            _n: &str,
            _f: Option<[u8; 16]>,
            _l: usize,
            _incl: bool,
        ) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(Vec::new())
        }
        fn bundle(
            &self,
            _n: &str,
            _q: Option<&str>,
            _k: Option<MemoryKindTag>,
            _w: Option<(i64, i64)>,
            _l: usize,
        ) -> Result<Vec<MemoryEntry>, MemoryError> {
            match &self.err {
                Some(MemoryError::NotFound) => Err(MemoryError::NotFound),
                Some(MemoryError::DimMismatch(d)) => Err(MemoryError::DimMismatch(d.clone())),
                Some(MemoryError::EmbedderUnavailable) => Err(MemoryError::EmbedderUnavailable),
                Some(MemoryError::StaleIndex(d)) => Err(MemoryError::StaleIndex(d.clone())),
                Some(MemoryError::InvalidArgument(d)) => {
                    Err(MemoryError::InvalidArgument(d.clone()))
                }
                Some(MemoryError::Internal(d)) => Err(MemoryError::Internal(d.clone())),
                None => Ok(self.entries.clone()),
            }
        }
        fn decay(
            &self,
            _n: &str,
            _t: i64,
            _m: u32,
            _d: bool,
        ) -> Result<DecayReportEntry, MemoryError> {
            Ok(DecayReportEntry {
                candidates: Vec::new(),
                evicted: 0,
                kept: 0,
                dry_run: true,
            })
        }
        fn stats(&self, _n: &str) -> Result<MemoryStatsEntry, MemoryError> {
            Ok(MemoryStatsEntry::default())
        }
        fn restore(&self, _n: &str, _id: &[u8; 32]) -> Result<bool, MemoryError> {
            Ok(false)
        }
        fn forget(&self, _n: &str, _id: &[u8; 32]) -> Result<bool, MemoryError> {
            Ok(false)
        }
    }

    fn entry(tag: u8, text: &str) -> MemoryEntry {
        MemoryEntry {
            memory_id: [tag; 32],
            content: text.as_bytes().to_vec(),
            kind: MemoryKindTag::Episodic,
            instance_id: [0u8; 16],
            created_ms: 0,
            dim: 3,
            access_count: 0,
            last_accessed_ms: 0,
            tombstoned_ms: None,
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
    fn bundles_ordered_entries_and_drops_score_and_created_ms() {
        let cap = ConsolidateCapability::new(
            StubView::ok(vec![
                entry(1, "the deadline is march 3rd"),
                entry(2, "the client prefers email"),
            ]),
            "mem::local-dev".into(),
        );
        let out = cap.invoke(&req(br#"{"k":10}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let entries = v["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(v["count"], 2);
        assert_eq!(entries[0]["text"], "the deadline is march 3rd");
        assert_eq!(entries[0]["ref"], ContentRef::from_bytes([1; 32]).to_hex());
        let s = String::from_utf8_lossy(&out);
        assert!(
            !s.contains("score"),
            "no similarity score in the committed bundle (SN-8)"
        );
        assert!(
            !s.contains("created_ms"),
            "created_ms is not committed (off-hash)"
        );
    }

    #[test]
    fn empty_bundle_returns_a_note_not_an_error() {
        let cap = ConsolidateCapability::new(StubView::ok(Vec::new()), "mem::local-dev".into());
        let out = cap.invoke(&req(b"{}")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["count"], 0);
        assert!(v["note"].as_str().unwrap().contains("no episodic memories"));
    }

    #[test]
    fn soft_fails_every_recoverable_error() {
        for err in [
            MemoryError::NotFound,
            MemoryError::DimMismatch("d".into()),
            MemoryError::EmbedderUnavailable,
            MemoryError::StaleIndex("s".into()),
            MemoryError::InvalidArgument("a".into()),
        ] {
            let cap = ConsolidateCapability::new(StubView::fail(err), "mem::local-dev".into());
            let out = cap.invoke(&req(br#"{"query":"launch"}"#)).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(
                v["count"], 0,
                "a recoverable error soft-fails to an empty bundle"
            );
            assert!(v["note"].is_string());
        }
    }

    #[test]
    fn internal_error_dead_letters() {
        let cap = ConsolidateCapability::new(
            StubView::fail(MemoryError::Internal("boom".into())),
            "mem::local-dev".into(),
        );
        assert!(
            cap.invoke(&req(b"{}")).is_err(),
            "a hard backend fault dead-letters the chain"
        );
    }

    #[test]
    fn refuses_unknown_arg_and_bad_json() {
        let cap = ConsolidateCapability::new(StubView::ok(Vec::new()), "mem::local-dev".into());
        assert!(
            cap.invoke(&req(br#"{"nope":1}"#)).is_err(),
            "unknown arg refused"
        );
        assert!(cap.invoke(&req(b"not json")).is_err(), "bad json refused");
    }

    #[test]
    fn tool_def_is_flat_builtin_readback_no_egress() {
        let def = consolidate_tool_def();
        assert_eq!(def.tool_id.0, "consolidate");
        assert_eq!(def.tool_version.0, "1");
        assert!(matches!(def.kind, ToolKind::Builtin));
        assert!(matches!(def.idempotency_class, IdempotencyClass::Readback));
        assert!(matches!(
            def.required_capability.net_scope_required,
            NetScope::None
        ));
    }
}
