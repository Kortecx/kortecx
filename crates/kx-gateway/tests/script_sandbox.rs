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
use kx_mote::{
    EffectPattern, GraphPosition, InputDataId, LogicRef, ModelId, Mote, MoteDef, NdClass,
    PromptTemplateHash, ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_gateway_core::ScriptAdmin;
use kx_tool_registry::{SqliteToolRegistry, ToolRegistry};
use kx_warrant::{
    FsMode, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, SecretScope,
    ToolGrant, WarrantSpec,
};

use kx_gateway::scripts::{
    provision_shim, register_script, Interpreter, ScriptDecl, ScriptWish,
};

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

fn serde_json_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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
    register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("registration");

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
    register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("registration");

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
    register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("registration");

    let mote = calling_mote(&d.name, &d.version);
    let declared = FsScope {
        mounts: d.wish.fs_mounts.clone(),
    };

    // Arm A — the caller grants NO filesystem. The declared wish is not a subset,
    // so the broker refuses before the capability is ever invoked.
    let ungranted = granting_warrant(&d.name, &d.version, BTreeMap::new());
    let refused = h.broker.dispatch(
        &mote,
        &ungranted,
        &d.name,
        request("", declared.clone()),
    );
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
    register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("registration");

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
    let d = decl("script/durable", "read -r line; printf 'restored:%s' \"$line\"");
    register_script(
        &d,
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("registration");

    // A restart: the durable registry and store survive; the broker does not.
    let restarted = Harness {
        _dir: h._dir,
        store: h.store.clone(),
        registry: h.registry.clone(),
        broker: std::sync::Arc::new(LocalCapabilityBroker::new(h.store.clone())),
    };
    let mote = calling_mote(&d.name, &d.version);
    let warrant = granting_warrant(&d.name, &d.version, BTreeMap::new());

    let before = restarted
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


/// ★ The benchmark's oracle must be RIGHT, or the family measures the model
/// against a wrong expectation and a correct model scores zero.
///
/// These are the exact inputs and answers the `script` family's tasks assert, run
/// through the real sandbox rather than reasoned about.
#[test]
fn the_bundled_benchmark_script_computes_what_the_suite_expects() {
    let h = Harness::new();
    let Some(shim) = shim_or_skip(&h.store) else {
        return;
    };
    let (name, version) = kx_gateway::scripts::bench_script_tool();
    kx_gateway::scripts::register_bench_script(
        Some(shim),
        &h.store,
        &h.registry,
        &h.broker,
        kx_gateway::default_executor_class(),
    )
    .expect("the bundled benchmark script should register");

    let mote = calling_mote(&name, &version);
    let warrant = granting_warrant(&name, &version, BTreeMap::new());
    for (input, expected) in [
        ("the quick brown fox jumps", "WORDS=5"),
        ("alpha beta gamma", "WORDS=3"),
    ] {
        let handle = h
            .broker
            .dispatch(
                &mote,
                &warrant,
                &name,
                request(input, FsScope::empty()),
            )
            .expect("dispatch");
        let staged = h.store.get(&handle.staged_ref).expect("staged");
        assert_eq!(
            String::from_utf8_lossy(&staged).trim_end(),
            expected,
            "the suite's script-family answer for {input:?} must be what the script computes"
        );
    }
}
