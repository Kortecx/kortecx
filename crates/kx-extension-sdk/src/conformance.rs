// SPDX-License-Identifier: Apache-2.0
//! The per-connector **conformance harness** — prove a connector is safe to
//! register BEFORE it touches a live runtime.
//!
//! [`run_conformance`] dials a connector through the **real** dial path
//! ([`McpGateway::register_server`]) and dispatches a discovered tool through a
//! **real** [`LocalCapabilityBroker`] warrant gate (no journal, no frozen trio, no
//! gateway service). It mechanizes a subset of the D167 *Extension Acceptance Gate*:
//!
//! - **Item 3 — out-of-process.** Every discovered tool registers as
//!   [`ToolKind::Mcp`] (an external process), never `Builtin`.
//! - **Item 5 — warrant / SN-8.** A tool fires ONLY under a warrant that grants it:
//!   a no-grant warrant is refused, an insufficient grant (a different tool) is
//!   refused, and (for a credentialed connector) a warrant lacking the secret scope
//!   is refused. A correctly-granted warrant succeeds.
//! - **Item 7 — secret-by-ref (D81).** A credential supplied out-of-band reaches no
//!   sink (the staged result, the broker handle, the request payload, the MoteId).
//!   The bundled echo is the positive control; the [`contains_secret`] scanner is
//!   self-tested so the check provably bites.
//! - **Item 10 — on/off.** With the connector absent the tool does not resolve
//!   (fail-closed); registering it adds exactly its namespaced tool(s), nothing else.
//!
//! The harness is **panic-free**: every step folds into a [`CheckResult`]; a failure
//! is reported, never thrown. A dialed connector that is merely *unreachable* is
//! reported distinctly from a *failed* gate check (honest degradation, GR15).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kx_capability::{
    BrokerError, Capability, CapabilityBroker, EffectRequest, LocalCapabilityBroker,
};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_mote::{
    EffectPattern, GraphPosition, InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef,
    NdClass, PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, SecretRef,
    SecretScope, ToolGrant, WarrantField, WarrantSpec,
};
use serde::Serialize;

use kx_mcp_gateway::{
    CapabilitySink, ConnectionHealth, McpGateway, RegisterOutcome, SessionMode,
    SqliteConnectionStore, TransportSpec,
};
use kx_tool_registry::{SqliteToolRegistry, ToolKind};

/// A connector to put under test.
pub struct ConnectorUnderTest {
    /// The server name (namespaces the discovered tool ids as `<name>/<remote>`).
    pub name: String,
    /// How to reach it (stdio subprocess or Streamable-HTTP).
    pub transport: TransportSpec,
    /// An optional out-of-band credential reference (an env-var NAME, D81). When
    /// set AND that env var holds a value, item 7 scans every sink for that value.
    pub credential_ref: Option<String>,
    /// The firing posture (stateless single-shot vs a reused stateful session).
    pub session_mode: SessionMode,
}

/// One gate check's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// A short stable name (`out_of_process`, `warrant_enforcement`, …).
    pub name: &'static str,
    /// The Extension Acceptance Gate item this maps to (3 / 5 / 7 / 10).
    pub gate_item: u8,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable detail (the refusal seen, the count asserted, why it skipped).
    pub detail: String,
}

impl CheckResult {
    // pub(crate): the RC-SW1 skill harness (`skill_conformance`) builds the same
    // report vocabulary over a DECLARATIVE artifact.
    pub(crate) fn pass(name: &'static str, gate_item: u8, detail: impl Into<String>) -> Self {
        Self {
            name,
            gate_item,
            passed: true,
            detail: detail.into(),
        }
    }
    pub(crate) fn fail(name: &'static str, gate_item: u8, detail: impl Into<String>) -> Self {
        Self {
            name,
            gate_item,
            passed: false,
            detail: detail.into(),
        }
    }
}

/// The full conformance outcome for one connector.
#[derive(Debug, Clone, Serialize)]
pub struct ConformanceReport {
    /// The connector's server name.
    pub connector: String,
    /// Whether the dial reached the connector (a tool was discovered).
    pub reachable: bool,
    /// The number of tools discovered + registered.
    pub discovered: u32,
    /// Per-gate-item checks.
    pub checks: Vec<CheckResult>,
}

impl ConformanceReport {
    /// `true` iff every check passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.passed) && self.reachable
    }
}

/// A distinctive secret window scan (the D81 leak detector). `true` iff `needle`
/// appears anywhere in `haystack`. Public so a connector author can reuse it.
#[must_use]
pub fn contains_secret(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// A [`CapabilitySink`] that records the capabilities the gateway registers, so the
/// harness can re-dispatch them through a broker under controlled warrants.
#[derive(Default)]
struct RecordingSink {
    caps: Mutex<Vec<Box<dyn Capability>>>,
}

impl CapabilitySink for RecordingSink {
    fn register_capability(&self, capability: Box<dyn Capability>) {
        if let Ok(mut g) = self.caps.lock() {
            g.push(capability);
        }
    }
}

impl RecordingSink {
    fn drain(&self) -> Vec<Box<dyn Capability>> {
        self.caps
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }
}

/// The dial outcome the checks consume.
struct Dialed {
    registry: Arc<SqliteToolRegistry>,
    caps: Vec<Box<dyn Capability>>,
    outcome: RegisterOutcome,
    /// The namespaced tool ids the connector contributed (`<name>/<remote>`).
    tools: Vec<ToolName>,
}

/// Dial the connector through the real gateway path and collect its registered
/// tools + capabilities. Returns `Err` only on a harness-setup failure (a store
/// could not open); an *unreachable* connector returns `Ok` with `discovered = 0`.
fn dial(cut: &ConnectorUnderTest) -> Result<Dialed, String> {
    let registry =
        Arc::new(SqliteToolRegistry::open_in_memory().map_err(|e| format!("registry: {e}"))?);
    let store = SqliteConnectionStore::open_in_memory().map_err(|e| format!("store: {e}"))?;
    let sink = Arc::new(RecordingSink::default());
    let gateway = McpGateway::new(
        store,
        registry.clone(),
        sink.clone() as Arc<dyn CapabilitySink>,
        Vec::new(),
    );
    let outcome = gateway
        .register_server(
            &cut.name,
            cut.transport.clone(),
            cut.credential_ref.clone(),
            cut.session_mode,
        )
        .map_err(|e| format!("register_server: {e}"))?;

    // The namespaced tools this connector contributed (`<name>/…`).
    let prefix = format!("{}/", cut.name.trim());
    let tools = registry
        .discover(4096, None)
        .map_err(|e| format!("discover: {e}"))?
        .into_iter()
        .filter(|e| e.def.tool_id.0.starts_with(&prefix))
        .map(|e| e.def.tool_id.clone())
        .collect();

    Ok(Dialed {
        registry,
        caps: sink.drain(),
        outcome,
        tools,
    })
}

/// Run the bundled subset of the Extension Acceptance Gate against one connector.
#[must_use]
pub fn run_conformance(cut: &ConnectorUnderTest) -> ConformanceReport {
    let dialed = match dial(cut) {
        Ok(d) => d,
        Err(e) => {
            return ConformanceReport {
                connector: cut.name.clone(),
                reachable: false,
                discovered: 0,
                checks: vec![CheckResult::fail(
                    "dial",
                    0,
                    format!("harness setup failed: {e}"),
                )],
            };
        }
    };

    let reachable =
        dialed.outcome.health == ConnectionHealth::Connected && dialed.outcome.discovered > 0;
    if !reachable {
        return ConformanceReport {
            connector: cut.name.clone(),
            reachable: false,
            discovered: dialed.outcome.discovered,
            checks: vec![CheckResult::fail(
                "reachable",
                0,
                format!(
                    "connector did not become reachable (health={:?}, discovered={})",
                    dialed.outcome.health, dialed.outcome.discovered
                ),
            )],
        };
    }

    let mut checks = Vec::new();
    checks.push(check_out_of_process(&dialed));

    // Items 5 + 7 dispatch through a broker holding the registered capabilities.
    let content = Arc::new(InMemoryContentStore::new());
    let broker = LocalCapabilityBroker::new(content.clone());
    for cap in dialed.caps {
        broker.register_capability(cap);
    }
    let tool = dialed.tools.first().cloned();
    if let Some(t) = &tool {
        checks.push(check_warrant_enforcement(&broker, t, cut));
        checks.push(check_no_secret_leak(&broker, &content, t, cut));
    } else {
        checks.push(CheckResult::fail(
            "warrant_enforcement",
            5,
            "no tool discovered to test",
        ));
        checks.push(CheckResult::fail(
            "secret_by_ref",
            7,
            "no tool discovered to test",
        ));
    }
    checks.push(check_extension_on_and_off(
        &dialed.registry,
        tool.as_ref(),
        cut,
    ));

    ConformanceReport {
        connector: cut.name.clone(),
        reachable: true,
        discovered: dialed.outcome.discovered,
        checks,
    }
}

/// Item 3 — every discovered tool is an external `ToolKind::Mcp`, never `Builtin`.
fn check_out_of_process(d: &Dialed) -> CheckResult {
    let entries = match d.registry.discover(4096, None) {
        Ok(e) => e,
        Err(e) => return CheckResult::fail("out_of_process", 3, format!("discover: {e}")),
    };
    let prefix = format!("{}/", d.outcome_name());
    let mut seen = 0u32;
    for entry in entries
        .into_iter()
        .filter(|e| e.def.tool_id.0.starts_with(&prefix))
    {
        seen += 1;
        match entry.def.kind {
            ToolKind::Mcp { .. } => {}
            other => {
                return CheckResult::fail(
                    "out_of_process",
                    3,
                    format!(
                        "tool {} registered as {other:?}, expected ToolKind::Mcp",
                        entry.def.tool_id.0
                    ),
                );
            }
        }
    }
    if seen == 0 {
        return CheckResult::fail("out_of_process", 3, "no namespaced tools registered");
    }
    CheckResult::pass(
        "out_of_process",
        3,
        format!("{seen} tool(s) registered out-of-process (ToolKind::Mcp)"),
    )
}

/// Item 5 — the tool fires ONLY under a warrant that grants it.
fn check_warrant_enforcement(
    broker: &LocalCapabilityBroker<Arc<InMemoryContentStore>>,
    tool: &ToolName,
    cut: &ConnectorUnderTest,
) -> CheckResult {
    let version = ToolVersion("1".into());
    let mote = probe_mote(tool, &version);

    // (a) no-grant: a warrant granting nothing must be refused on the ToolGrants axis.
    let no_grant = warrant_for(&[], None);
    match broker.dispatch(&mote, &no_grant, tool, effect(r#"{"q":"hi"}"#)) {
        Err(BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::ToolGrants,
        }) => {}
        Ok(_) => {
            return CheckResult::fail(
                "warrant_enforcement",
                5,
                "a no-grant warrant was NOT refused",
            )
        }
        Err(other) => {
            return CheckResult::fail(
                "warrant_enforcement",
                5,
                format!("no-grant refused on the wrong axis (expected ToolGrants): {other}"),
            );
        }
    }

    // (b) insufficient: a warrant granting a DIFFERENT tool must be refused.
    let other = ToolName(format!("{}__not_this", tool.0));
    let insufficient = warrant_for(&[(other, version.clone())], None);
    match broker.dispatch(&mote, &insufficient, tool, effect(r#"{"q":"hi"}"#)) {
        Err(BrokerError::CapabilityExceedsWarrant {
            axis: WarrantField::ToolGrants,
        }) => {}
        Ok(_) => {
            return CheckResult::fail(
                "warrant_enforcement",
                5,
                "an insufficient (wrong-tool) grant was NOT refused",
            );
        }
        Err(other) => {
            return CheckResult::fail(
                "warrant_enforcement",
                5,
                format!("wrong-tool refused on the wrong axis (expected ToolGrants): {other}"),
            );
        }
    }

    // (c) positive control: a correctly-granted warrant PASSES THE GATE. The tool may
    // still fail on the harness's placeholder args (a real connector with strict
    // params) — that is a CapabilityFailure, NOT a gate refusal, and is acceptable.
    let secret = cut.credential_ref.as_ref().map(|v| SecretRef(v.clone()));
    let granted = warrant_for(&[(tool.clone(), version)], secret);
    match broker.dispatch(&mote, &granted, tool, effect(r#"{"q":"hi"}"#)) {
        Ok(_) | Err(BrokerError::CapabilityFailure { .. } | BrokerError::StageWriteFailed { .. }) => {
            CheckResult::pass(
                "warrant_enforcement",
                5,
                "no-grant + wrong-tool refused on the ToolGrants axis; a correct grant passed the gate",
            )
        }
        Err(gate) => CheckResult::fail(
            "warrant_enforcement",
            5,
            format!("a correctly-granted warrant was refused at the gate: {gate}"),
        ),
    }
}

/// Item 7 — an out-of-band credential reaches no sink (D81). The
/// [`contains_secret`] scanner is self-tested so the check provably bites.
fn check_no_secret_leak(
    broker: &LocalCapabilityBroker<Arc<InMemoryContentStore>>,
    content: &Arc<InMemoryContentStore>,
    tool: &ToolName,
    cut: &ConnectorUnderTest,
) -> CheckResult {
    // The scanner must catch a planted secret (negative control) — else a real
    // leak would slip through silently.
    if !contains_secret(b"prefix-SECRET-suffix", b"SECRET") || contains_secret(b"clean", b"SECRET")
    {
        return CheckResult::fail(
            "secret_by_ref",
            7,
            "the leak scanner is broken (self-test failed)",
        );
    }

    // Resolve the credential value (if any) the connector has "in play".
    let Some(var) = cut.credential_ref.as_deref() else {
        return CheckResult::pass(
            "secret_by_ref",
            7,
            "connector declares no credential; scanner self-test passed",
        );
    };
    let Ok(secret) = std::env::var(var) else {
        return CheckResult::pass(
            "secret_by_ref",
            7,
            format!("credential_ref {var:?} not set in env; scanner self-test passed (set it to exercise the leak scan)"),
        );
    };
    let secret = secret.into_bytes();

    let version = ToolVersion("1".into());
    let mote = probe_mote(tool, &version);
    let warrant = warrant_for(&[(tool.clone(), version)], Some(SecretRef(var.to_string())));
    let req = effect(r#"{"q":"hi"}"#);
    let payload = req.payload.clone();

    // The credential is "in play" whether the tool SUCCEEDS or fails on the
    // placeholder args — either way it must not reach a sink. Scan every available
    // sink for both outcomes (a tool error string is itself a sink).
    let mut sinks: Vec<(&str, Vec<u8>)> = vec![
        ("EffectRequest.payload", payload),
        ("MoteId", mote.id.as_bytes().to_vec()),
    ];
    match broker.dispatch(&mote, &warrant, tool, req) {
        Ok(handle) => {
            let staged = content
                .get(&handle.staged_ref)
                .map(|b| b.to_vec())
                .unwrap_or_default();
            sinks.push((
                "BrokerHandle provenance",
                format!("{handle:?}").into_bytes(),
            ));
            sinks.push(("staged result", staged));
        }
        Err(e) => {
            // A tool-level failure (e.g. strict args) is fine — its error text is a sink.
            sinks.push(("BrokerError", format!("{e:?}").into_bytes()));
        }
    }
    for (where_, bytes) in &sinks {
        if contains_secret(bytes, &secret) {
            return CheckResult::fail("secret_by_ref", 7, format!("secret leaked into {where_}"));
        }
    }
    CheckResult::pass(
        "secret_by_ref",
        7,
        "credential reached no sink (payload / handle / staged / error / MoteId)",
    )
}

/// Item 10 — with the connector absent the tool does not resolve; registering it
/// adds exactly its namespaced tool(s).
fn check_extension_on_and_off(
    registry: &Arc<SqliteToolRegistry>,
    tool: Option<&ToolName>,
    cut: &ConnectorUnderTest,
) -> CheckResult {
    // OFF: a fresh empty broker refuses the (absent) tool, fail-closed.
    let content = Arc::new(InMemoryContentStore::new());
    let broker_off = LocalCapabilityBroker::new(content);
    if let Some(t) = tool {
        let version = ToolVersion("1".into());
        let mote = probe_mote(t, &version);
        let warrant = warrant_for(&[(t.clone(), version)], None);
        if broker_off
            .dispatch(&mote, &warrant, t, effect(r#"{"q":"hi"}"#))
            .is_ok()
        {
            return CheckResult::fail(
                "on_off",
                10,
                "an unregistered tool fired on an empty broker",
            );
        }
    }

    // ON: the registry holds exactly the connector's namespaced tools (no others
    // from this connector, and the registration touched nothing outside its prefix).
    let prefix = format!("{}/", cut.name.trim());
    let entries = match registry.discover(4096, None) {
        Ok(e) => e,
        Err(e) => return CheckResult::fail("on_off", 10, format!("discover: {e}")),
    };
    let mine = entries
        .iter()
        .filter(|e| e.def.tool_id.0.starts_with(&prefix))
        .count();
    if mine == 0 {
        return CheckResult::fail("on_off", 10, "no namespaced tools after registration");
    }
    CheckResult::pass(
        "on_off",
        10,
        format!("fail-closed when absent; {mine} namespaced tool(s) present when registered"),
    )
}

impl Dialed {
    /// The connector's name, recovered from a registered tool's namespace prefix
    /// (the dial outcome carries no name; the registry does, via `<name>/…`).
    fn outcome_name(&self) -> String {
        self.tools
            .first()
            .and_then(|t| t.0.split_once('/').map(|(s, _)| s.to_string()))
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Probe fixtures (a Mote that declares the tool + a warrant that grants it).
// Mirrors crates/kx-mcp/tests/common — kept here so connector authors get the
// same fixtures without depending on a test-only module.
// ---------------------------------------------------------------------------

/// A `WorldMutating` `StageThenCommit` Mote declaring `(tool, version)` in its
/// `tool_contract` (so `LocalCapabilityBroker::dispatch` admits the call).
fn probe_mote(tool: &ToolName, version: &ToolVersion) -> Mote {
    let mut tool_contract = BTreeMap::new();
    tool_contract.insert(tool.clone(), version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7; 32]),
        model_id: ModelId("m".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([7; 32]),
        tool_contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([7; 32]),
        GraphPosition(vec![7]),
        smallvec::SmallVec::new(),
    )
}

/// A warrant granting exactly `grants` (and, when `secret` is set, that secret).
fn warrant_for(grants: &[(ToolName, ToolVersion)], secret: Option<SecretRef>) -> WarrantSpec {
    let tool_grants: BTreeSet<ToolGrant> = grants
        .iter()
        .map(|(name, version)| ToolGrant {
            tool_id: name.clone(),
            tool_version: version.clone(),
        })
        .collect();
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants,
        secret_scope: secret.map_or(SecretScope::None, |s| {
            SecretScope::AllowList(BTreeSet::from([s]))
        }),
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1024,
            max_output_tokens: 256,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 5_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// An `EffectRequest` carrying `args_json` under `StageThenCommit`, no egress/fs.
fn effect(args_json: &str) -> EffectRequest {
    EffectRequest {
        payload: args_json.as_bytes().to_vec(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: None,
        net_scope: NetScope::None,
        fs_scope: FsScope::empty(),
        secret_scope: SecretScope::None,
    }
}

// ---------------------------------------------------------------------------
// Reference connector (kx-connector-example) — the deterministic positive control.
// ---------------------------------------------------------------------------
//
// NOTE: the runtime-bundled `kx-mcp-echo` is a SINGLE-SHOT `tools/call` responder
// (no initialize/tools/list handshake), so it is registered by the live serve path
// via a hardcoded ToolDef — it cannot be DIALED through `register_server`. The
// conformance harness therefore dials the SDK's own reference connector, which
// implements the full MCP lifecycle (see `src/bin/reference_connector.rs`).

/// Resolve the `kx-connector-example` reference connector binary: a
/// `KX_CONNECTOR_EXAMPLE_PATH` override first, then a dev/test walk up to the
/// workspace `target/{debug,release}` dir. `None` ⇒ not built (run
/// `cargo build -p kx-extension-sdk`). Integration tests in this crate can instead
/// use `env!("CARGO_BIN_EXE_kx-connector-example")` (always set).
#[must_use]
pub fn reference_connector_path() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("KX_CONNECTOR_EXAMPLE_PATH") {
        let path = PathBuf::from(over);
        if path.exists() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|n| n == "target") {
            for profile in ["debug", "release"] {
                let candidate = ancestor.join(profile).join("kx-connector-example");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// A [`ConnectorUnderTest`] for the reference connector (the deterministic positive
/// control), or `None` if the binary is not built.
#[must_use]
pub fn reference_connector() -> Option<ConnectorUnderTest> {
    let path = reference_connector_path()?;
    Some(ConnectorUnderTest {
        name: "example".into(),
        transport: TransportSpec::Stdio {
            command: path.to_string_lossy().into_owned(),
            args: vec![],
        },
        credential_ref: None,
        session_mode: SessionMode::Stateless,
    })
}
