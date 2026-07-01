// SPDX-License-Identifier: Apache-2.0
//! RC5a (durable memory) — the bundled `remember@1` capability, its typed [`ToolDef`],
//! and the serve-broker registration. A model in a `kx/recipes/react-memory` ReAct turn
//! proposes `{"content": <fact to remember>, "kind": "semantic"|"episodic"}`; this
//! durably records the fact in the run's namespace (content-addressed, idempotent) so a
//! LATER run can `recall@1` it. The committed observation is the memory-id receipt.
//!
//! # Boundaries (load-bearing)
//!
//! - **Honest write classification.** `IdempotencyClass::Token` (verdict #3 — a write
//!   is NOT a read-probe). The write is content-addressed + `INSERT OR IGNORE`, so it
//!   is `IdempotentByConstruction`: a re-dispatch (exactly-once pre-commit replay) is a
//!   durable no-op, and Token SELF-CLOSES so the HITL gate AUTO-PROCEEDS (no egress /
//!   no spend / reversible via forget — the safety rationale for the gate does not apply).
//! - **No egress.** `NetScope::None` / `FsScope::empty()` — the store is reached via the
//!   in-process `Arc<dyn MemoryView>`.
//! - **Server-injected namespace.** BOUND at registration to the serve's primary
//!   principal (verdict #5) — a model NEVER scopes it.
//! - **Fail-SOFT on a bad write.** A recoverable error (oversize/stale/dim/etc.) returns
//!   an honest `stored:false` observation the model reads + recovers from; only a hard
//!   `MemoryError::Internal` returns `Err` and dead-letters the chain.

use std::sync::Arc;

use kx_capability::{Capability, CapabilityFailureReason, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore};
use kx_gateway_core::{MemoryError, MemoryKindTag, MemoryView, MemoryWrite};
use kx_mote::{EffectPattern, ToolName, ToolVersion};
use kx_tool_registry::{IdempotencyClass, InputSchema, ParamSpec, ParamType, ToolDef, ToolKind};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolRequirement};
use serde::{Deserialize, Serialize};

/// remember mutates the durable memory sidecar and commits its bytes (the receipt) as a
/// `result_ref` — the stage-then-commit content-addressed path (idempotent write).
const PATTERNS: &[EffectPattern] = &[EffectPattern::StageThenCommit];

/// The max remembered payload (bytes) — a fact is a short note (mirrors the store bound).
const MAX_CONTENT: usize = 8 * 1024;

/// The bundled memory-write capability (`remember@1`), bound to the serve's memory
/// namespace (verdict #5 — the model never scopes it).
pub(crate) struct RememberCapability {
    name: ToolName,
    version: ToolVersion,
    memory: Arc<dyn MemoryView>,
    namespace: String,
}

impl RememberCapability {
    pub(crate) fn new(memory: Arc<dyn MemoryView>, namespace: String) -> Self {
        Self {
            name: ToolName("remember".into()),
            version: ToolVersion("1".into()),
            memory,
            namespace,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberArgs {
    content: String,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Serialize)]
struct Observation {
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_id: Option<String>,
    stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

fn encode(obs: &Observation) -> Result<Vec<u8>, CapabilityFailureReason> {
    serde_json::to_vec(obs)
        .map_err(|e| CapabilityFailureReason::Other(format!("remember: encode: {e}")))
}

fn parse_kind(kind: Option<&str>) -> MemoryKindTag {
    match kind {
        Some("episodic") => MemoryKindTag::Episodic,
        _ => MemoryKindTag::Semantic,
    }
}

impl Capability for RememberCapability {
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
        let args: RememberArgs = serde_json::from_slice(&request.payload)
            .map_err(|e| CapabilityFailureReason::Other(format!("remember: bad args: {e}")))?;
        // Defensive clamp (the typed schema + the store also bound it).
        if args.content.len() > MAX_CONTENT {
            return encode(&Observation {
                memory_id: None,
                stored: false,
                note: Some(format!("not stored: content exceeds {MAX_CONTENT} bytes")),
            });
        }
        let write = MemoryWrite {
            namespace: &self.namespace,
            content: args.content.as_bytes(),
            embedding: None, // the host embeds the content
            kind: parse_kind(args.kind.as_deref()),
            // An in-run write's run provenance lives on the committed remember-action
            // Mote (the journaled fact); the sidecar denormalizes an all-zero instance.
            instance_id: [0u8; 16],
        };
        match self.memory.store(write) {
            Ok(out) => encode(&Observation {
                memory_id: Some(ContentRef::from_bytes(out.memory_id).to_hex()),
                stored: true,
                note: (!out.inserted).then(|| "already remembered (deduped)".to_string()),
            }),
            // SOFT-FAIL every recoverable error → an honest not-stored observation.
            Err(
                MemoryError::NotFound
                | MemoryError::DimMismatch(_)
                | MemoryError::EmbedderUnavailable
                | MemoryError::StaleIndex(_)
                | MemoryError::InvalidArgument(_),
            ) => encode(&Observation {
                memory_id: None,
                stored: false,
                note: Some(
                    "not stored (no embedder, or an incompatible/invalid memory)".to_string(),
                ),
            }),
            // A hard backend fault is NOT recoverable ⇒ dead-letter honestly.
            Err(MemoryError::Internal(detail)) => Err(CapabilityFailureReason::Other(format!(
                "remember: {detail}"
            ))),
        }
    }
}

/// The bundled remember tool's identity — `remember@1` (a FLAT builtin id).
#[must_use]
pub(crate) fn remember_tool() -> (ToolName, ToolVersion) {
    (ToolName("remember".into()), ToolVersion("1".into()))
}

/// The `remember@1` [`ToolDef`]: a memory-write tool with NO egress / NO fs scope, a
/// typed `{content, kind?}` schema (unknown keys refused), and `Token` (the honest
/// idempotent-by-construction class — auto-proceeds the HITL gate; verdict #3).
#[must_use]
pub(crate) fn remember_tool_def() -> ToolDef {
    let (tool_id, tool_version) = remember_tool();
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
        description: "Remember a durable fact for LATER runs to recall. Arg: {\"content\": <fact to remember>, \"kind\": \"semantic\"|\"episodic\" (optional)}. Content-addressed + idempotent (remembering the same fact twice is a no-op). Use it to persist what you learn.".into(),
        idempotency_class: IdempotencyClass::Token,
        input_schema: Some(InputSchema {
            params: vec![
                ParamSpec {
                    name: "content".into(),
                    ty: ParamType::Str {
                        max_len: MAX_CONTENT,
                    },
                    required: true,
                },
                ParamSpec {
                    name: "kind".into(),
                    ty: ParamType::Str { max_len: 16 },
                    required: false,
                },
            ],
            deny_unknown: true,
        }),
    }
}

/// Register the bundled [`RememberCapability`] (`remember@1`) on the serve broker over
/// the live `Arc<dyn MemoryView>`, bound to the serve's memory `namespace`.
pub(crate) fn register_remember_capability<S: ContentStore + Send + Sync>(
    broker: &LocalCapabilityBroker<S>,
    memory: Arc<dyn MemoryView>,
    namespace: String,
) {
    broker.register_capability(Box::new(RememberCapability::new(memory, namespace)));
    tracing::info!("RC5a: remember@1 capability registered (kx/recipes/react-memory)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use kx_gateway_core::{MemoryEntry, MemoryHitEntry, StoreMemoryOutcome};
    use kx_warrant::SecretScope;

    struct StubView {
        outcome: Option<StoreMemoryOutcome>,
        err: Option<MemoryError>,
    }
    impl StubView {
        fn ok(inserted: bool) -> Arc<dyn MemoryView> {
            Arc::new(Self {
                outcome: Some(StoreMemoryOutcome {
                    memory_id: [7; 32],
                    inserted,
                    dim: 3,
                }),
                err: None,
            })
        }
        fn fail(err: MemoryError) -> Arc<dyn MemoryView> {
            Arc::new(Self {
                outcome: None,
                err: Some(err),
            })
        }
    }
    impl MemoryView for StubView {
        fn store(&self, _w: MemoryWrite<'_>) -> Result<StoreMemoryOutcome, MemoryError> {
            match &self.err {
                Some(MemoryError::Internal(d)) => Err(MemoryError::Internal(d.clone())),
                Some(MemoryError::InvalidArgument(d)) => {
                    Err(MemoryError::InvalidArgument(d.clone()))
                }
                Some(MemoryError::EmbedderUnavailable) => Err(MemoryError::EmbedderUnavailable),
                Some(MemoryError::StaleIndex(d)) => Err(MemoryError::StaleIndex(d.clone())),
                Some(MemoryError::DimMismatch(d)) => Err(MemoryError::DimMismatch(d.clone())),
                Some(MemoryError::NotFound) => Err(MemoryError::NotFound),
                None => Ok(self.outcome.unwrap()),
            }
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
            _include_tombstoned: bool,
        ) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(Vec::new())
        }
        fn bundle(
            &self,
            _n: &str,
            _q: Option<&str>,
            _k: Option<kx_gateway_core::MemoryKindTag>,
            _w: Option<(i64, i64)>,
            _l: usize,
        ) -> Result<Vec<MemoryEntry>, MemoryError> {
            Ok(Vec::new())
        }
        fn decay(
            &self,
            _n: &str,
            _ttl_ms: i64,
            _min_access: u32,
            _dry_run: bool,
        ) -> Result<kx_gateway_core::DecayReportEntry, MemoryError> {
            Ok(kx_gateway_core::DecayReportEntry {
                candidates: Vec::new(),
                evicted: 0,
                kept: 0,
                dry_run: true,
            })
        }
        fn stats(&self, _n: &str) -> Result<kx_gateway_core::MemoryStatsEntry, MemoryError> {
            Ok(kx_gateway_core::MemoryStatsEntry::default())
        }
        fn restore(&self, _n: &str, _id: &[u8; 32]) -> Result<bool, MemoryError> {
            Ok(false)
        }
        fn forget(&self, _n: &str, _id: &[u8; 32]) -> Result<bool, MemoryError> {
            Ok(false)
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
    fn stores_a_fact_and_returns_the_receipt() {
        let cap = RememberCapability::new(StubView::ok(true), "mem::local-dev".into());
        let out = cap
            .invoke(&req(br#"{"content":"the deadline is march 3rd"}"#))
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stored"], true);
        assert_eq!(v["memory_id"], ContentRef::from_bytes([7; 32]).to_hex());
    }

    #[test]
    fn dedup_hit_is_stored_true_with_a_note() {
        let cap = RememberCapability::new(StubView::ok(false), "mem::x".into());
        let out = cap.invoke(&req(br#"{"content":"x"}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stored"], true);
        assert!(v["note"].as_str().unwrap().contains("deduped"));
    }

    #[test]
    fn recoverable_error_soft_fails_to_not_stored() {
        let cap = RememberCapability::new(
            StubView::fail(MemoryError::EmbedderUnavailable),
            "mem::x".into(),
        );
        let out = cap.invoke(&req(br#"{"content":"x"}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stored"], false);
        assert!(v["note"].is_string());
    }

    #[test]
    fn hard_internal_error_dead_letters() {
        let cap = RememberCapability::new(
            StubView::fail(MemoryError::Internal("x".into())),
            "mem::x".into(),
        );
        assert!(cap.invoke(&req(br#"{"content":"x"}"#)).is_err());
    }

    #[test]
    fn refuses_unknown_arg_and_missing_content() {
        let cap = RememberCapability::new(StubView::ok(true), "mem::x".into());
        assert!(cap.invoke(&req(br#"{"content":"x","evil":1}"#)).is_err());
        assert!(cap.invoke(&req(br"{}")).is_err());
    }

    #[test]
    fn tool_def_is_flat_builtin_token() {
        let (name, ver) = remember_tool();
        assert_eq!(name.0, "remember");
        assert!(!name.0.contains('/'));
        assert_eq!(ver.0, "1");
        let def = remember_tool_def();
        assert!(matches!(def.kind, ToolKind::Builtin));
        assert_eq!(
            def.idempotency_class,
            IdempotencyClass::Token,
            "remember is an honest idempotent-by-construction WRITE (verdict #3)"
        );
        assert_eq!(def.required_capability.net_scope_required, NetScope::None);
    }
}
