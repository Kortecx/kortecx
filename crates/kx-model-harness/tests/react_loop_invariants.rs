//! PR-4 (M5) — "the model drives a tool-call `ReAct` loop" with a REAL GGUF model
//! (run with `--features with-model`, host + Metal). The live counterpart of
//! `react_loop_e2e.rs`. A small model may or may not emit a valid tool-call
//! envelope, so this asserts only the HARD invariants that hold for ANY output;
//! the feedback / fail-closed / crash-resume paths are covered DETERMINISTICALLY
//! in `react_loop_e2e.rs` (a real model's choices cannot be scripted reproducibly).
//!
//! Hard invariants (hold for ANY model output):
//! - the loop **completes** (never a panic / abort / hang), `turns_used >= 1`, and
//!   `turns_used <= max_turns` / `tool_calls <= max_tool_calls` (bounded);
//! - the outcome is one of `Answered` / `BudgetExhausted` / `DeadLettered`;
//! - **R49** — two cold re-folds of the committed journal reproduce the byte-identical
//!   committed-facts digest, and it equals the live run's digest (every turn output +
//!   observation is a replayed fact, never re-sampled).

#![cfg(feature = "with-model")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    clippy::doc_markdown
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kx_capability::{CapabilityBroker, LocalCapabilityBroker, INSTANCE_ID_LEN};
use kx_journal::SqliteJournal;
use kx_mcp::{McpCapability, McpTransport, TransportError};
use kx_model_harness::{harness_warrant, model_id_for, Harness, ReactBudget, ReactStop};
use kx_mote::{ToolName, ToolVersion};
use kx_runtime::config::Mode;
use kx_runtime::{digest_journal, RuntimeConfig};
use kx_tool_registry::{
    IdempotencyClass, InMemoryToolRegistry, McpEndpointId, ToolDef, ToolKind, ToolProvenance,
    ToolRegistry,
};
use kx_warrant::{FsScope, NetScope, ResourceCeiling, ToolGrant, ToolRequirement};

const INSTRUCTION: &str = "You have a tool `mcp-tool`. To call it, output ONLY a JSON \
object of the shape {\"tool_call\":{\"name\":\"mcp-tool\",\"version\":\"1\",\"args\":{\"q\":\"...\"}}}. \
Otherwise reply with a short final answer. What is the capital of France?";

fn gguf() -> std::path::PathBuf {
    kx_model_harness::default_gguf_path()
}

fn config(dir: &Path) -> RuntimeConfig {
    RuntimeConfig {
        journal_path: dir.join("j.sqlite"),
        content_root: dir.join("c"),
        mode: Mode::Run,
        crash_at: None,
        checkpoint_every: None,
        audit_log: None,
    }
}

/// A const in-process MCP transport (no subprocess / network) — the real
/// `StdioTransport`/`HttpTransport` are exercised by the kx-mcp crate tests.
struct ConstTransport;
impl McpTransport for ConstTransport {
    fn round_trip(
        &self,
        _request: &[u8],
        _max: usize,
        _ms: u64,
        _idempotency_key: Option<&[u8; 32]>,
    ) -> Result<Vec<u8>, TransportError> {
        Ok(
            br#"{"jsonrpc":"2.0","id":1,"result":{"obs":"Paris is the capital of France."}}"#
                .to_vec(),
        )
    }
}

fn tool() -> ToolName {
    ToolName("mcp-tool".to_string())
}
fn version() -> ToolVersion {
    ToolVersion("1".to_string())
}

#[test]
fn react_loop_completes_is_bounded_and_replays_identically() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let model_id = model_id_for(&gguf()).expect("hash the gguf");
    let harness = Harness::open(&cfg, &gguf(), model_id.clone()).expect("open harness");

    // A warrant that grants the MCP tool (so assemble emits the menu) with enough
    // output budget for an envelope or a short answer.
    let mut warrant = harness_warrant(&model_id, 128, 30_000);
    warrant.tool_grants.insert(ToolGrant {
        tool_id: tool(),
        tool_version: version(),
    });

    let mut reg = InMemoryToolRegistry::with_builtins();
    let _ = reg.register(
        ToolDef {
            tool_id: tool(),
            tool_version: version(),
            kind: ToolKind::Mcp {
                endpoint: McpEndpointId("inproc://const".into()),
                remote_name: "echo".into(),
            },
            required_capability: ToolRequirement {
                net_scope_required: NetScope::None,
                fs_scope_required: FsScope::empty(),
                syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
                min_resource_ceiling: ResourceCeiling {
                    cpu_milli: 0,
                    mem_bytes: 0,
                    wall_clock_ms: 0,
                    fd_count: 0,
                    disk_bytes: 0,
                },
            },
            description: "Answer a question about a place.".into(),
            idempotency_class: IdempotencyClass::Staged,
            input_schema: None,
        },
        ToolProvenance::HumanAuthored {
            author: "test".into(),
        },
    );
    let registry: Arc<dyn ToolRegistry> = Arc::new(reg);

    let tb = LocalCapabilityBroker::new(harness.store.clone());
    tb.register_capability(Box::new(McpCapability::new(
        tool(),
        version(),
        McpEndpointId("inproc://const".into()),
        "echo",
        Box::new(ConstTransport),
    )));
    let tool_broker: Arc<dyn CapabilityBroker> = Arc::new(tb);

    let budget = ReactBudget {
        max_turns: 3,
        max_tool_calls: 2,
    };
    let outcome = harness
        .drive_react_loop(
            &cfg,
            &warrant,
            INSTRUCTION,
            registry,
            tool_broker,
            [0x5a; INSTANCE_ID_LEN],
            budget,
        )
        .expect("the ReAct loop completes (no panic/abort/hang)");

    // Bounded + a defined terminal outcome (holds for ANY model output).
    assert!(outcome.turns_used >= 1, "at least one turn ran");
    assert!(
        outcome.turns_used <= budget.max_turns,
        "bounded by max_turns"
    );
    assert!(
        outcome.tool_calls <= budget.max_tool_calls,
        "bounded by max_tool_calls"
    );
    assert!(matches!(
        outcome.outcome,
        ReactStop::Answered | ReactStop::BudgetExhausted | ReactStop::DeadLettered
    ));

    // R49: the live digest equals a cold re-fold, and two cold re-folds agree —
    // every committed turn output + observation is a replayed fact, never re-sampled.
    let d1 = digest_journal(&*harness.journal).expect("fold 1");
    std::thread::sleep(Duration::from_millis(1));
    let reopened = SqliteJournal::open(&cfg.journal_path).expect("reopen");
    let d2 = digest_journal(&reopened).expect("fold 2");
    assert_eq!(d1, outcome.run.digest, "live digest == cold re-fold digest");
    assert_eq!(d1, d2, "two cold re-folds agree (R49)");
}
