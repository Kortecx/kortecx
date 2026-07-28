// Integration-test file: compiled as a separate crate from the host lib;
// inherits workspace `[lints]` deny on `unwrap_used` / `expect_used` but tests
// legitimately use `.unwrap()` for fixture construction. The `pedantic` group is
// also allowed here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
#![cfg(feature = "embedded-worker")]
//! The script primitive, end to end, through the REAL broker and the REAL
//! platform sandbox.
//!
//! Nothing here is a stand-in. The tools are registered into a real
//! `SqliteToolRegistry`, dispatched through a real `LocalCapabilityBroker`
//! (so the shipped `precheck` is what decides authority), and executed by
//! `bwrap`/`sandbox-exec` with the bundled shim as the body.
//!
//! ## What each test would read if the change were absent
//!
//! - **It executed** — before this change every Mote took the passthrough
//!   fallback, which commits the Mote's *input*. Asserting the committed bytes
//!   are the script's *computed output* is an assertion passthrough cannot
//!   satisfy: the script transforms its input, so output and input differ.
//! - **Wish ∩ grants** — a script wanting more than its caller has is refused,
//!   and the SAME script under a sufficient grant fires. Either half alone is
//!   satisfiable by a broken gate (always-refuse passes the first; always-allow
//!   passes the second); only the pair pins it.
//! - **Fail-closed** — with no shim provisioned, registration refuses. A gate
//!   that let it register and then silently ran the script on the host would
//!   pass any test that only checked the output.
//!
//! These need the shim binary, so they runtime-skip when it is absent rather
//! than passing vacuously — a skip prints, a false green does not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_capability::{CapabilityBroker, EffectRequest, LocalCapabilityBroker};
use kx_content::{ContentRef, ContentStore, LocalFsContentStore};
use kx_gateway_core::ScriptAdmin;
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_tool_registry::{SqliteToolRegistry, ToolRegistry};
use kx_warrant::{
    FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, SecretScope, ToolGrant,
    WarrantSpec,
};

use kx_gateway::scripts::{provision_shim, register_script, Interpreter, ScriptDecl, ScriptWish};

/// The shim must be built for these to mean anything. Runtime-skip (loudly)
/// rather than pass with nothing exercised.
fn shim_or_skip(store: &LocalFsContentStore) -> Option<ContentRef> {
    let provisioned = provision_shim(store);
    if provisioned.is_none() {
        eprintln!(
            "SKIP: the kx-script-runner shim is not built — run \
             `cargo build -p kx-script-runner --bins` (or set KX_SCRIPT_RUNNER_PATH)"
        );
    }
    provisioned
}

struct Harness {
    _dir: tempfile::TempDir,
    store: LocalFsContentStore,
    registry: std::sync::Arc<SqliteToolRegistry>,
    broker: std::sync::Arc<LocalCapabilityBroker<LocalFsContentStore>>,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalFsContentStore::open(dir.path().join("content")).unwrap();
        let registry =
            std::sync::Arc::new(SqliteToolRegistry::open(dir.path().join("tools.db")).unwrap());
        let broker = std::sync::Arc::new(LocalCapabilityBroker::new(store.clone()));
        Self {
            _dir: dir,
            store,
            registry,
            broker,
        }
    }

    /// The admin seam over the same live objects — the read path an operator and
    /// the restart-rehydration both use.
    fn admin(
        &self,
        shim: Option<kx_content::ContentRef>,
    ) -> kx_gateway::scripts::HostScriptRegistry<LocalFsContentStore> {
        kx_gateway::scripts::HostScriptRegistry::new(
            self.registry.clone(),
            self.store.clone(),
            self.broker.clone(),
            shim,
            kx_gateway::default_executor_class(),
        )
    }
}

fn tool(name: &str) -> (ToolName, ToolVersion) {
    (ToolName(name.into()), ToolVersion("1".into()))
}

/// A declaration wanting nothing beyond the runtime's own scratch.
fn decl(name: &str, source: &str) -> ScriptDecl {
    let (tool_id, version) = tool(name);
    ScriptDecl {
        name: tool_id,
        version,
        interpreter: Interpreter::Sh,
        source: source.as_bytes().to_vec(),
        description: "a test script".into(),
        author: "tests".into(),
        argv: Vec::new(),
        env: Vec::new(),
        wish: ScriptWish::default(),
    }
}

/// A Mote that names `capability` in its tool contract (the broker checks this
/// before it checks the warrant).
fn calling_mote(name: &ToolName, version: &ToolVersion) -> Mote {
    let mut contract = BTreeMap::new();
    contract.insert(name.clone(), version.clone());
    let def = MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([0; 32]),
        model_id: ModelId("local".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([0; 32]),
        tool_contract: contract,
        nd_class: NdClass::WorldMutating,
        config_subset: BTreeMap::new(),
        effect_pattern: EffectPattern::StageThenCommit,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: kx_mote::InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    };
    Mote::new(
        def,
        InputDataId::from_bytes([0; 32]),
        GraphPosition(b"root".to_vec()),
        smallvec::SmallVec::new(),
    )
}

/// A warrant granting `(name, version)` and exactly `mounts`.
fn granting_warrant(
    name: &ToolName,
    version: &ToolVersion,
    mounts: BTreeMap<PathBuf, FsMode>,
) -> WarrantSpec {
    let mut grants = BTreeSet::new();
    grants.insert(ToolGrant {
        tool_id: name.clone(),
        tool_version: version.clone(),
    });
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope { mounts },
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([0; 32]),
        tool_grants: grants,
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 30_000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: kx_gateway::default_executor_class(),
        secret_scope: SecretScope::None,
        ..Default::default()
    }
}

fn request(input: &str, fs_scope: FsScope) -> EffectRequest {
    EffectRequest {
        payload: format!(r#"{{"input":{}}}"#, serde_json_string(input)).into_bytes(),
        pattern: EffectPattern::StageThenCommit,
        idempotency_key: Some([7; 32]),
        net_scope: NetScope::None,
        fs_scope,
        secret_scope: SecretScope::None,
    }
}

/// Proper JSON string escaping. The hand-rolled version this replaces did not
/// escape control characters, so the first multi-line payload produced a
/// malformed tool call — which is worth noting beyond the test: a script taking
/// CSV gets it through the JSON tool-call envelope, and every newline has to
/// survive that trip.
fn serde_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// ★ The script RAN. Under the old passthrough the committed bytes are the
/// Mote's input; here they are the script's transformation of the caller's
/// input, which passthrough cannot produce.
#[test]
fn a_registered_script_executes_in_the_sandbox_and_returns_its_output() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    // Computes over its stdin (length + a marker) using only shell builtins, so
    // the output can never coincide with the input.
    let d = decl(
        "script/measure",
        "read -r line; printf 'measured:%s:%s' \"${#line}\" \"$line\"",
    );
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let handle = h
        .broker
        .dispatch(
            &mote,
            &warrant,
            &d.name,
            request("kortecx", FsScope::empty()),
        )
        .expect("dispatch");

    // The broker staged the capability's bytes; read them back by ref.
    let staged = h.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(
        String::from_utf8_lossy(&staged).trim_end(),
        "measured:7:kortecx",
        "the script's output should be its computation over stdin"
    );
    assert_ne!(
        staged.as_ref(),
        b"kortecx".as_slice(),
        "committing the INPUT is what passthrough does — the script did not run"
    );
}

/// ★ Confinement. A script may exec only what its grants cover. `rev` lives in
/// `/usr/bin`, which a `sh` script's plumbing does not mount, so the attempt
/// fails — and it fails as a REFUSAL, not as a silent empty result that would
/// read like a script which simply had nothing to say.
#[test]
fn a_script_cannot_exec_a_binary_outside_its_granted_directories() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let d = decl("script/escape-attempt", "printf 'abc' | rev");
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let got = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request("", FsScope::empty()));
    assert!(
        got.is_err(),
        "a script reached a binary outside its granted directories"
    );
}

/// ★ The pair. A wish wider than the caller's grant is refused; the SAME script
/// under a sufficient grant fires.
#[test]
fn a_wish_wider_than_the_grant_is_refused_and_the_same_script_fires_when_granted() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let wanted = tempfile::tempdir().unwrap();
    let wanted_path = std::fs::canonicalize(wanted.path()).unwrap();
    std::fs::write(wanted_path.join("data.txt"), "seven").unwrap();

    let mut d = decl(
        "script/reads-a-mount",
        &format!("cat {}/data.txt", wanted_path.display()),
    );
    d.wish
        .fs_mounts
        .insert(wanted_path.clone(), FsMode::ReadOnly);
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let declared = FsScope {
        mounts: d.wish.fs_mounts.clone(),
    };

    // Arm A — the caller grants NO filesystem. The declared wish is not a subset,
    // so the broker refuses before the capability is ever invoked.
    let ungranted = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let refused = h
        .broker
        .dispatch(&mote, &ungranted, &d.name, request("", declared.clone()));
    assert!(
        matches!(
            refused,
            Err(kx_capability::BrokerError::CapabilityExceedsWarrant { .. })
        ),
        "a wish outside the caller's grant must be refused, got {refused:?}"
    );

    // Arm B — the same script, the same wish, a caller that HAS the grant.
    let granted = granting_warrant(&d.name, &d.version, d.wish.fs_mounts.clone());
    let handle = h
        .broker
        .dispatch(&mote, &granted, &d.name, request("", declared))
        .expect("a granted script should fire");
    let staged = h.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(
        String::from_utf8_lossy(&staged).trim_end(),
        "seven",
        "the granted script should have read the mounted file"
    );
}

/// ★ Fail-closed. With no shim there is nothing that can run a script safely, so
/// registration refuses outright rather than registering something that would
/// fall back to the host.
#[test]
fn without_the_shim_a_script_does_not_register() {
    let h = Harness::new();
    let d = decl("script/never", "printf 'should not run'");
    let got = register_script(
        &d,
        None,
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    );
    assert!(got.is_err(), "a script registered with no sandbox shim");
    assert!(
        h.registry.lookup(&d.name, &d.version).is_none(),
        "a refused script must leave no registry row"
    );
}

/// An unknown interpreter is refused at admission, naming what is accepted.
#[test]
fn an_unknown_interpreter_is_refused_at_admission() {
    assert!(Interpreter::parse("perl").is_none());
    assert_eq!(Interpreter::parse("sh"), Some(Interpreter::Sh));
    assert_eq!(Interpreter::parse("python3"), Some(Interpreter::Python3));
    assert_eq!(Interpreter::parse("node"), Some(Interpreter::Node));
}

/// The registry pins the exact thing that will run, so a changed script is a
/// different registration rather than a silent substitution behind one name.
///
/// Asserted through the PUBLIC read path, because that is what an operator sees
/// and what the restart-rehydration walks — a check that reached into storage
/// directly could pass while the surface everyone actually uses was broken.
#[test]
fn the_registry_pins_the_exact_source_and_reads_it_back() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let d = decl("script/pinned", "printf 'v1'");
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let admin = h.admin(Some(shim));
    let (row, source) = admin
        .get(&d.name.0, &d.version.0)
        .expect("read")
        .expect("the registered script should be readable");
    assert_eq!(source, d.source, "the row must read back the exact source");
    assert_eq!(row.interpreter, "sh");
    assert_eq!(
        row.source_ref_hex,
        kx_content::ContentRef::of(&d.source).to_hex(),
        "the row must name the source's content ref"
    );
    // A tool that is not a script must not be readable through this surface, or a
    // caller could believe it had read a source that does not exist.
    assert!(
        admin.get("echo", "1").expect("read").is_none(),
        "a non-script tool must not be reported as a script"
    );
}

/// ★ A script survives a restart. The registry is durable and the broker is not,
/// so without rehydration a restarted serve resolves the tool and then fails at
/// dispatch — a row that reads as live and is not.
///
/// The pair is the assertion: a FRESH broker cannot fire the script, and the same
/// broker after rehydration can. Either half alone proves nothing.
#[test]
fn a_registered_script_is_fireable_again_after_a_restart() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let d = decl(
        "script/durable",
        "read -r line; printf 'restored:%s' \"$line\"",
    );
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    // A restart: the durable registry and store survive; the broker does not.
    let restarted = Harness {
        _dir: h._dir,
        store: h.store.clone(),
        registry: h.registry.clone(),
        broker: std::sync::Arc::new(LocalCapabilityBroker::new(h.store.clone())),
    };
    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());

    let before =
        restarted
            .broker
            .dispatch(&mote, &warrant, &d.name, request("x", FsScope::empty()));
    assert!(
        before.is_err(),
        "a fresh broker should not already know the script — this test would be \
         vacuous if it did"
    );

    restarted.admin(Some(shim)).rehydrate();

    let handle = restarted
        .broker
        .dispatch(&mote, &warrant, &d.name, request("x", FsScope::empty()))
        .expect("the script should fire again after rehydration");
    let staged = restarted.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(String::from_utf8_lossy(&staged).trim_end(), "restored:x");
}

/// ★ The benchmark's oracle must be RIGHT, or the family measures a correct model
/// against a wrong expectation and scores it zero.
///
/// These are the exact inputs and answers the `script` family's tasks assert, run
/// through the real sandbox rather than reasoned about — a shell aggregation loop
/// is precisely the kind of thing that looks obviously correct and is not.
#[test]
fn the_bundled_benchmark_script_computes_what_the_suite_expects() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let (name, version) = kx_gateway::scripts::bench_script_tool();
    if kx_gateway::scripts::register_bench_script(
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .is_none()
    {
        eprintln!("SKIP: this host cannot run a sandboxed script");
        return;
    }

    let mote = calling_mote(&name, &version);
    let warrant = granting_warrant(&name, &version, BTreeMap::new());
    for (input, expected) in [
        (
            "north,1372\nsouth,894\neast,2051\nwest,663\n\
             north,1189\nsouth,447\neast,1908\nwest,726",
            "ROWS=8 TOTAL=9250",
        ),
        ("alpha,120\nbeta,305\ngamma,75", "ROWS=3 TOTAL=500"),
        // A malformed row is skipped, not counted and not fatal — real CSVs have
        // headers and blank lines, and an oracle that dies on one is not usable.
        ("name,amount\nalpha,10\n\nbeta,5", "ROWS=2 TOTAL=15"),
        // A single row with no trailing newline: the shape that silently dropped
        // the last record before the loop was fixed to read it.
        ("solo,42", "ROWS=1 TOTAL=42"),
        ("", "ROWS=0 TOTAL=0"),
    ] {
        let handle = h
            .broker
            .dispatch(&mote, &warrant, &name, request(input, FsScope::empty()))
            .expect("dispatch");
        let staged = h.store.get(&handle.staged_ref).expect("staged");
        assert_eq!(
            String::from_utf8_lossy(&staged).trim_end(),
            expected,
            "the suite's script-family answer for {input:?} must be what the script computes"
        );
    }
}

/// Register a script, or skip the test when THIS HOST cannot sandbox.
///
/// Registration probes the declared interpreter through the real platform
/// sandbox, so `InterpreterUnavailable` means the host cannot run a confined
/// script at all — no `bwrap`, restricted user namespaces, no usable interpreter.
/// A CI runner without bwrap is that host, and these tests have nothing to say
/// there.
///
/// Deliberately narrow: ONLY that variant skips. Any other admission error is a
/// real refusal and still fails the test, so this cannot quietly turn the whole
/// suite into a no-op the way a blanket `is_err() => return` would.
fn register_or_skip(d: &ScriptDecl, shim: kx_content::ContentRef, h: &Harness) -> Option<()> {
    match register_script(
        d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    ) {
        Ok(_) => Some(()),
        Err(kx_gateway::scripts::ScriptAdmissionError::InterpreterUnavailable {
            interpreter,
            detail,
        }) => {
            eprintln!("SKIP: this host cannot run a sandboxed {interpreter} script ({detail})");
            None
        }
        Err(other) => {
            panic!("registration failed for a reason that is not a host limitation: {other}")
        }
    }
}

/// Build a declaration for a given interpreter, skipping when it is unavailable.
fn decl_for(name: &str, interpreter: Interpreter, source: &str) -> Option<ScriptDecl> {
    if interpreter.resolve().is_none() {
        eprintln!(
            "SKIP: {} is not installed on this host",
            interpreter.as_str()
        );
        return None;
    }
    let (tool_id, version) = tool(name);
    Some(ScriptDecl {
        name: tool_id,
        version,
        interpreter,
        source: source.as_bytes().to_vec(),
        description: "a real-workload test script".into(),
        author: "tests".into(),
        argv: Vec::new(),
        env: Vec::new(),
        wish: ScriptWish {
            wall_clock_ms: 30_000,
            ..ScriptWish::default()
        },
    })
}

/// ★ node on a real workload — JSON in, aggregation, JSON out.
///
/// node was in the interpreter allowlist and had never executed. It is also the
/// interpreter most likely to be installed by a version manager rather than a
/// system package, so this doubles as proof that discovery finds one.
#[test]
fn node_transforms_real_json() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let source = r#"
let raw = "";
process.stdin.on("data", (c) => (raw += c));
process.stdin.on("end", () => {
  const orders = JSON.parse(raw);
  const byStatus = {};
  for (const o of orders) {
    byStatus[o.status] = (byStatus[o.status] || 0) + o.total;
  }
  process.stdout.write(JSON.stringify(byStatus, Object.keys(byStatus).sort()));
});
"#;
    let Some(d) = decl_for("script/order-totals", Interpreter::Node, source) else {
        return;
    };
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let orders = r#"[{"status":"paid","total":40},{"status":"open","total":15},{"status":"paid","total":2}]"#;
    let handle = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request(orders, FsScope::empty()))
        .expect("the node script should fire");
    let staged = h.store.get(&handle.staged_ref).expect("staged");
    let text = String::from_utf8_lossy(&staged);
    assert!(
        text.contains("\"paid\":42") && text.contains("\"open\":15"),
        "node did not aggregate the JSON correctly: {text}"
    );
}

/// ★ Egress is denied unless granted — as a PAIR, because "the fetch failed" is
/// also what a wrong URL, a down host or a typo produces.
///
/// Arm A: a script with no egress in its wish cannot open a socket. Arm B: the
/// same script, with the loopback host in its wish AND in the caller's warrant,
/// reaches a listener started by the test. Only the pair separates *denied* from
/// *never worked*.
#[test]
fn egress_is_denied_unless_the_caller_granted_it() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    // A local listener that answers one line, so "reachable" is a real outcome
    // rather than an assumption about the outside world.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut s in listener.incoming().take(4).flatten() {
            use std::io::Write as _;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHELLO");
        }
    });

    let source = format!(
        r#"
const net = require("net");
const sock = net.connect({port}, "127.0.0.1");
sock.on("data", (d) => {{ process.stdout.write("REACHED"); sock.end(); }});
sock.on("error", () => {{ process.stdout.write("BLOCKED"); process.exit(0); }});
sock.setTimeout(4000, () => {{ process.stdout.write("BLOCKED"); process.exit(0); }});
"#
    );
    let Some(mut d) = decl_for("script/egress-probe", Interpreter::Node, &source) else {
        return;
    };

    // Arm A — no egress declared, none granted.
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }
    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let handle = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request("", FsScope::empty()))
        .expect("the script itself should run; only its socket should be refused");
    let denied = h.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(
        String::from_utf8_lossy(&denied).trim(),
        "BLOCKED",
        "a script with no egress in its wish opened a socket"
    );

    // Arm B — the same script, egress declared AND granted.
    d.name = ToolName("script/egress-granted".into());
    d.wish.net_hosts = [kx_warrant::Host("127.0.0.1".into())].into_iter().collect();
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }
    let mote = calling_mote(&d.name, &d.version);
    let mut granted = granting_warrant(&d.name, &d.version, BTreeMap::new());
    granted.net_scope = d.wish_net_scope_for_test();
    let mut req = request("", FsScope::empty());
    req.net_scope = granted.net_scope.clone();
    let handle = h
        .broker
        .dispatch(&mote, &granted, &d.name, req)
        .expect("a granted script should fire");
    let reached = h.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(
        String::from_utf8_lossy(&reached).trim(),
        "REACHED",
        "a script whose caller granted the host could not reach it — arm A's \
         BLOCKED would then prove nothing"
    );
}

/// ★ A ceiling this host cannot apply is REFUSED at registration, not accepted and
/// ignored.
///
/// The failure this closes: an egress allowlist naming a real host was accepted on every
/// platform, stored, listed with its declared scope — and then, on Linux, run with the
/// network completely open, because confining egress per host needs a firewall the
/// runtime has no privileges to install. Nothing refused it and nothing said so, which
/// made the declaration read as a constraint at every layer above the one that dropped it.
///
/// Asserted through the real `register_script`, not the probe's own predicate: a unit
/// test of the predicate would pass whether or not anything ever consulted it.
#[test]
fn a_ceiling_this_host_cannot_enforce_is_refused_at_registration() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let Some(mut d) = decl_for("script/unenforceable", Interpreter::Sh, "printf ok") else {
        return;
    };
    // A host that is not loopback: no platform here can confine a body to it.
    d.wish.net_hosts = [kx_warrant::Host("api.example.com".into())]
        .into_iter()
        .collect();

    let refused = register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    );
    let Err(kx_gateway::scripts::ScriptAdmissionError::UnenforceableCeiling(why)) = refused else {
        panic!(
            "a script declaring egress to a host this sandbox cannot confine must be \
             REFUSED — accepting it publishes a ceiling that constrains nothing"
        );
    };
    assert!(
        why.contains("api.example.com"),
        "the refusal must name what it could not honour: {why}"
    );

    // And the refusal is SCOPED: drop the unenforceable axis and the same script
    // registers. Without this half, the test would also pass if registration had simply
    // stopped working altogether.
    //
    // This half needs a host that can actually run a sandboxed script, and the refusal
    // above does not — it is checked before the interpreter is ever probed, which is why
    // the first half runs everywhere. On a host with no usable sandboxed interpreter the
    // control is skipped rather than asserted, matching every other test in this file:
    // asserting it would report "this machine has no bwrap" as "the refusal is too
    // broad", which is a different claim entirely.
    d.wish.net_hosts.clear();
    if register_or_skip(&d, shim, &h).is_none() {
        eprintln!(
            "SKIP (control half): this host cannot register a sandboxed script at all, so \
             'the same script without the unenforceable axis still registers' is unprovable \
             here. The refusal itself was asserted above."
        );
    }
}

/// ★ The output cap at a REALISTIC size. A cap tested at four bytes says nothing
/// about a cap at a megabyte: the buffering, the pipe behaviour and the bounded
/// read are all different at scale.
#[test]
fn the_output_cap_holds_at_a_realistic_size() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    // ~2 MiB of real output, against a 1 MiB declared cap.
    let source = "i=0; while [ $i -lt 2048 ]; do \
                  printf '%01024d' $i; i=$((i+1)); done";
    let mut d = decl("script/floods", source);
    d.wish.max_output_bytes = 1024 * 1024;
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let got = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request("", FsScope::empty()));
    assert!(got.is_err(), "2 MiB of output passed a 1 MiB cap");

    // And a payload just under the cap still succeeds — otherwise "it refused"
    // could simply mean large outputs never work.
    let mut ok_decl = decl(
        "script/large-but-ok",
        "i=0; while [ $i -lt 256 ]; do \
                            printf '%01024d' $i; i=$((i+1)); done",
    );
    ok_decl.wish.max_output_bytes = 1024 * 1024;
    if register_or_skip(&ok_decl, shim, &h).is_none() {
        return;
    }
    let mote = calling_mote(&ok_decl.name, &ok_decl.version);
    let warrant = granting_warrant(&ok_decl.name, &ok_decl.version, BTreeMap::new());
    let handle = h
        .broker
        .dispatch(
            &mote,
            &warrant,
            &ok_decl.name,
            request("", FsScope::empty()),
        )
        .expect("a 256 KiB output under a 1 MiB cap should succeed");
    let staged = h.store.get(&handle.staged_ref).expect("staged");
    assert_eq!(staged.as_ref().len(), 256 * 1024);
}

/// ★ Concurrent dispatches must not collide. Every run gets its own scratch
/// directories and its own descriptor; if any of that were shared, parallel
/// scripts would read each other's input or overwrite each other's result — and a
/// serial test cannot see it.
#[test]
fn concurrent_script_dispatches_do_not_collide() {
    let h = std::sync::Arc::new(Harness::new());
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let d = decl(
        "script/echoes-its-input",
        "read -r line; printf 'got:%s' \"$line\"",
    );
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mut handles = Vec::new();
    for i in 0..8 {
        let h = std::sync::Arc::clone(&h);
        let name = d.name.clone();
        let version = d.version.clone();
        handles.push(std::thread::spawn(move || {
            let mote = calling_mote(&name, &version);
            let warrant = granting_warrant(&name, &version, BTreeMap::new());
            let input = format!("caller-{i}");
            let handle = h
                .broker
                .dispatch(&mote, &warrant, &name, request(&input, FsScope::empty()))
                .expect("dispatch");
            let staged = h.store.get(&handle.staged_ref).expect("staged");
            (i, String::from_utf8_lossy(&staged).trim_end().to_string())
        }));
    }
    for handle in handles {
        let (i, output) = handle.join().expect("thread");
        assert_eq!(
            output,
            format!("got:caller-{i}"),
            "a concurrent run saw another caller's input"
        );
    }
}

/// ★ A script that produces output and THEN fails must not have its partial
/// output committed as a successful result. This is the shape most likely to
/// mislead an agent: bytes that look like an answer, from a run that broke.
#[test]
fn partial_output_before_a_failure_is_not_committed() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let d = decl(
        "script/fails-late",
        "printf 'looks like a real answer'; exit 7",
    );
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let got = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request("", FsScope::empty()));
    assert!(
        got.is_err(),
        "a script that exited 7 had its partial output accepted as an answer"
    );
}

/// ★ A declared time budget must actually STOP a runaway script.
///
/// The axis a fast script can never exercise: an aggregation returns in
/// milliseconds, so a missing timeout and a working one look identical. Here the
/// script sleeps far past its budget, so the two are distinguishable — enforced
/// means a quick failure, ignored means the call blocks for the whole sleep and
/// then SUCCEEDS.
#[test]
fn a_runaway_script_is_stopped_by_its_declared_time_budget() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let mut d = decl("script/runaway", "sleep 30; printf 'finished anyway'");
    d.wish.wall_clock_ms = 2_000;
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let started = std::time::Instant::now();
    let got = h
        .broker
        .dispatch(&mote, &warrant, &d.name, request("", FsScope::empty()));
    let elapsed = started.elapsed();

    assert!(
        got.is_err(),
        "a script that ran 15x its budget was allowed to finish"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "the budget did not stop it — the call took {elapsed:?}, near the script's \
         own 30s sleep rather than its 2s budget"
    );
}

/// ★ A REAL workload, on a real interpreter, over a real file.
///
/// The shell tests run an interpreter whose prefix needs no mount at all, so the
/// interpreter-prefix and ancestor-metadata grants that python and node depend on
/// went unexercised. A shell one-liner also cannot show whether an interpreter can
/// reach its own standard library through the sandbox, which is the thing most
/// likely to be wrong.
#[test]
fn python3_processes_a_real_csv_from_a_mounted_directory() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    if Interpreter::Python3.resolve().is_none() {
        eprintln!("SKIP: python3 is not installed on this host");
        return;
    }

    let data = tempfile::tempdir().unwrap();
    let data_dir = std::fs::canonicalize(data.path()).unwrap();
    std::fs::write(
        data_dir.join("sales.csv"),
        "region,units,unit_price\n\
         north,12,4.50\n\
         south,7,4.50\n\
         north,3,10.00\n\
         east,20,1.25\n",
    )
    .unwrap();

    let source = format!(
        r#"
import csv, json, collections
totals = collections.defaultdict(float)
with open("{}/sales.csv", newline="") as fh:
    for row in csv.DictReader(fh):
        totals[row["region"]] += int(row["units"]) * float(row["unit_price"])
print(json.dumps({{"revenue": {{k: round(v, 2) for k, v in sorted(totals.items())}}}}))
"#,
        data_dir.display()
    );

    let (tool_id, version) = tool("script/revenue-by-region");
    let d = ScriptDecl {
        name: tool_id,
        version,
        interpreter: Interpreter::Python3,
        source: source.into_bytes(),
        description: "Aggregate revenue per region from the sales CSV.".into(),
        author: "tests".into(),
        argv: Vec::new(),
        env: Vec::new(),
        wish: ScriptWish {
            fs_mounts: [(data_dir.clone(), FsMode::ReadOnly)].into_iter().collect(),
            wall_clock_ms: 30_000,
            ..ScriptWish::default()
        },
    };
    if register_or_skip(&d, shim, &h).is_none() {
        return;
    }

    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, d.wish.fs_mounts.clone());
    let handle = h
        .broker
        .dispatch(
            &mote,
            &warrant,
            &d.name,
            request(
                "",
                FsScope {
                    mounts: d.wish.fs_mounts.clone(),
                },
            ),
        )
        .expect("the python3 script should fire");

    let staged = h.store.get(&handle.staged_ref).expect("staged");
    let text = String::from_utf8_lossy(&staged);
    // north = 12*4.50 + 3*10.00 = 84.0; south = 31.5; east = 25.0
    assert!(
        text.contains("\"north\": 84.0")
            && text.contains("\"south\": 31.5")
            && text.contains("\"east\": 25.0"),
        "python3 did not aggregate the mounted CSV correctly: {text}"
    );
}
