//! The real-model oracle benchmark witness (Tier-B, LOCAL): drive the `bench-v1` task
//! slice on a LIVE served model and score every run with the golden oracle scorers via
//! `kx_gateway::eval_bench` — the proof that the runtime's agentic quality is MEASURED on
//! real model output, not replayed from scripted fixtures.
//!
//! This is the hard-but-LOCAL gate. It is `#[ignore]` + `--features inference` and NEVER
//! runs in `just ci` (the deterministic golden gate is `just eval`, flake-proof over
//! fixtures). The oracle FLOORS are asserted only when a CAPABLE model is served
//! (Gemma-4-12B — Ollama `gemma3:12b` or a Gemma GGUF); on the weak Qwen3 CI stand-in the
//! run is RECORD-ONLY (numbers persisted, no assert) — a weak model must never gate the
//! real oracle.
//!
//! Drive BOTH engines (restart-per-run):
//!   `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b just eval-bench`   # Ollama
//!   `just fetch-gemma-model && KX_SERVE_MODEL_GGUF=<gemma-4-12b.gguf> just eval-bench`  # llama.cpp
//! Capture/refresh the committed per-engine baseline with `KX_BENCH_UPDATE_BASELINE=1`.

#![cfg(feature = "inference")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::eval_bench::score_live_suite;
use kx_gateway::{start, REACT_AUTO_RECIPE_HANDLE, REACT_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The per-task settle budget (a 12B model on CPU can take minutes per task).
///
/// Sized for the LONGEST chain in the corpus on the SLOWEST engine, not for the typical
/// task. At 240s the in-process llama.cpp arm timed out on a three-turn chain and the whole
/// suite aborted with `NotSettled` — a task that was working correctly, just not finishing
/// inside a budget set when nothing chained past two turns. A timeout that truncates a
/// legitimate chain does not measure the runtime, it measures the timeout, and it fails in
/// the most misleading way available: as a capability failure. Typical tasks still settle
/// in ~90s on that engine, so this is headroom rather than a slower run.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(600);
/// The aggregate `task_success` floor a CAPABLE model must clear (per-mille). Store-only
/// kv facts + a deterministic echo make the oracle a genuine tool-use proof; a capable
/// Gemma clears these comfortably (observed 1000/1000 on gemma3:12b). Set below the
/// observed value so single-task real-model nondeterminism never false-fails the floor.
const CAPABLE_TASK_SUCCESS_FLOOR: u32 = 600;
/// The baseline ratchet tolerance (per-mille). Real-model runs are nondeterministic — one
/// of five tasks flipping a binary metric swings the aggregate ~200 per-mille — so the
/// fail-closed regression gate absorbs that much noise while still catching a real
/// capability collapse (a gate falling far below the committed baseline).
const BASELINE_TOLERANCE: u32 = 200;

fn serve_gguf() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
}

/// Whether the operator opted Ollama in (`KX_SERVE_OLLAMA` truthy).
fn ollama_opted_in() -> bool {
    matches!(
        std::env::var("KX_SERVE_OLLAMA")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "on" | "true" | "yes"
    )
}

/// The (engine, model) label pair from the operator's env — the record's identity.
fn engine_and_model() -> (String, String) {
    if ollama_opted_in() {
        let model = std::env::var("KX_SERVE_OLLAMA_MODELS")
            .unwrap_or_default()
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        ("ollama".to_string(), model)
    } else {
        let model = serve_gguf()
            .and_then(|p| p.file_name().and_then(|f| f.to_str()).map(str::to_string))
            .unwrap_or_default();
        ("llamacpp".to_string(), model)
    }
}

/// `git rev-parse HEAD`, or `"unknown"` — the record's commit label (every recorded
/// number carries a commit + environment label).
fn git_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The run's human environment label.
///
/// `KX_EVAL_ENV_LABEL` PREFIXES the derived label rather than replacing it. The override
/// exists so a shared runner can say which runner it was, and a replacement would have
/// let that name stand in for the host facts it cannot know — a label that hides what it
/// is labelling. (It was set by the real-model workflow and read nowhere, which is the
/// same failure in a quieter form: a knob that does nothing reads exactly like a knob
/// that works.)
fn env_label(engine: &str, model: &str) -> String {
    let cores = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let derived = format!(
        "{}/{} ({cores} cores) | {engine} | {model}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    match std::env::var("KX_EVAL_ENV_LABEL") {
        Ok(tag) if !tag.trim().is_empty() => format!("{} | {derived}", tag.trim()),
        _ => derived,
    }
}

/// Drop every env var this run set, on every exit path.
///
/// The host allowlist in particular must not outlive the run: it is a deny-by-default
/// security control, and a leaked one would silently narrow — or, read the other way,
/// appear to authorize — whatever ran next in the same process.
fn clear_bench_env() {
    for k in [
        "KX_SERVE_TOOL_HOST_ALLOWLIST",
        "KX_SERVE_TOOL_DEADLINE_SECS",
        common::bench_http::BENCH_HTTP_CRED_ENV,
    ] {
        std::env::remove_var(k);
    }
}

/// The committed per-engine Gemma baseline (the fail-closed ratchet), in the kx-eval corpus.
fn baseline_path(engine: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../kx-eval/corpus/bench-v1")
        .join(format!("baseline.{engine}.json"))
}

/// The gitignored env-labelled trend sink at the repo root.
fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/benchmarks")
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(c) = KxGatewayClient::connect(endpoint.clone()).await {
            return c;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client connects to the gateway at {endpoint}");
}

/// Read the served model id from any journaled react turn — the post-run identity check
/// (guards against a stale/wrong serve reporting the expected label).
async fn served_model_id(c: &mut KxGatewayClient<Channel>) -> Option<String> {
    let turns = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(50),
            instance_id: None,
            step_salt: None,
        })
        .await
        .ok()?
        .into_inner();
    turns
        .turns
        .into_iter()
        .map(|t| t.model_id)
        .find(|m| !m.is_empty())
}

/// One server-embed document (empty embedding ⇒ the host embeds `content`).
fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

/// The grounding corpus the `reach` retrieval task searches. The load-bearing fact
/// (`ZEPHYR-77`) exists ONLY here — no model knows it — so an answer carrying it is proof
/// the run reached the dataset, and the distractors make the retrieval non-trivial.
const REACH_CORPUS: [&[u8]; 61] = [
    // THE fact. Everything below it is designed to be retrieved INSTEAD.
    b"The Helios ground station uses the callsign ZEPHYR-77 for all eclipse-window transmissions.",
    // ---- Near misses: right entity, wrong attribute. ------------------------------
    // Each of these is about Helios and about callsigns or eclipse windows, so they
    // rank high on any embedding of the question, and each carries a DIFFERENT
    // callsign. An answer built from one of them is confidently wrong.
    b"The Helios ground station uses the callsign ZEPHYR-31 for routine daylight telemetry.",
    b"The Helios ground station uses the callsign VESPER-77 for emergency traffic only.",
    b"The Helios ground station's eclipse-window transmissions are logged in bay 14.",
    b"The Helios ground station retired the callsign ZEPHYR-12 after the 2019 refit.",
    b"Helios eclipse-window transmissions are scheduled by the Meridian control desk.",
    b"The Helios ground station's backup transmitter is licensed under callsign ZEPHYR-78.",
    b"Eclipse-window transmissions from Helios are relayed through the Tarn repeater.",
    b"The Helios ground station uses no callsign at all for internal loopback tests.",
    // ---- Near misses: right attribute, wrong entity. -------------------------------
    // Same sentence shape, same callsign convention, different station.
    b"The Perihelion ground station uses the callsign ZEPHYR-64 for all eclipse-window transmissions.",
    b"The Anvil Bay ground station uses the callsign TERRAPIN-77 for all eclipse-window transmissions.",
    b"The Kestrel Flats ground station uses the callsign ZEPHYR-90 for eclipse-window transmissions.",
    b"The Lowfield ground station uses the callsign ORRERY-77 for all eclipse-window transmissions.",
    b"The Selene ground station uses the callsign ZEPHYR-77B for all eclipse-window transmissions.",
    b"The Tessellate ground station uses the callsign MARLIN-77 for eclipse-window transmissions.",
    // ---- Same domain, unrelated facts. ---------------------------------------------
    b"Ground station antennas are de-iced on a rolling twelve-hour schedule in winter.",
    b"An eclipse window is the interval during which a satellite passes through planetary shadow.",
    b"Callsign assignments are reviewed annually by the spectrum coordination board.",
    b"The Helios site was commissioned in 1974 and rebuilt after a fire in 1998.",
    b"Downlink budgets are computed from elevation angle, rain fade, and receiver noise figure.",
    b"Telemetry framing uses a CCSDS-derived packet format with Reed-Solomon outer coding.",
    b"Station keeping burns are planned to minimise propellant while holding the assigned slot.",
    b"The ground segment operations handbook is revision 11, issued in March.",
    b"Antenna pointing models are recalibrated against radio stars twice per year.",
    b"A transmission window closes when the spacecraft elevation drops below five degrees.",
    b"Uplink power is reduced automatically when the receiver reports margin above nine decibels.",
    b"The Meridian control desk hands over to the night shift at 22:00 local time.",
    b"Spectrum licences for the eclipse band are renewed on a five-year cycle.",
    b"The Tarn repeater was upgraded to solid-state amplifiers in the last maintenance window.",
    b"Signal acquisition typically takes under forty seconds from predicted rise.",
    b"Doppler compensation is applied in software at the baseband processor.",
    b"The site's emergency generator carries the full load for eighteen hours.",
    b"Weather radar data is ingested to predict rain fade on the Ka-band links.",
    b"Operators log every transmission with its start time, duration, and callsign.",
    b"The archive retains raw baseband recordings for ninety days before downsampling.",
    b"Cable losses between the feed and the low-noise amplifier are measured quarterly.",
    b"A hot standby receiver takes over automatically on primary failure.",
    b"Range and range-rate measurements are folded into the orbit determination filter.",
    b"The station's timing reference is disciplined by a hydrogen maser.",
    b"Scheduling conflicts are resolved by priority class, then by first request.",
    b"Antenna slew rates are limited to protect the elevation drive gearbox.",
    b"Link budgets assume a three-decibel implementation loss end to end.",
    b"The site perimeter is monitored by infrared cameras on a ten-second sweep.",
    b"Firmware on the baseband processor is updated only during scheduled outages.",
    b"Every commanded uplink is authenticated before the transmitter is keyed.",
    b"The eclipse season for this orbit runs for roughly six weeks twice a year.",
    b"Thermal cycling during eclipse is the dominant stressor on the solar array hinges.",
    b"Battery depth of discharge is held below forty percent through eclipse.",
    b"The spacecraft transitions to a low-power mode when the array current collapses.",
    b"Ranging tones are suppressed during eclipse to conserve downlink power.",
    b"A missed acquisition triggers an automatic retry on the next predicted pass.",
    b"The operations team reviews anomaly reports at the Tuesday morning board.",
    b"Configuration changes require a signed change request and a back-out plan.",
    // ---- Off-topic, the original distractors. --------------------------------------
    b"Tectonic plates drift over the mantle, causing earthquakes at their boundaries.",
    b"The mitochondria is the powerhouse of the cell, producing ATP from glucose.",
    b"Sourdough fermentation depends on a stable culture of wild yeast and lactobacilli.",
    b"The Bessemer process made cheap steel possible by blowing air through molten pig iron.",
    b"Migratory terns navigate using a combination of sun compass and magnetic sensing.",
    b"A well-tempered scale divides the octave into twelve equal logarithmic steps.",
    b"Mangrove roots trap sediment and buffer coastlines against storm surge.",
    b"The printing press reduced the cost of a book by more than an order of magnitude.",
];

/// The durable fact the `reach` memory task must recall. Written by the operator before
/// the run (all-zero instance) so the recall crosses a run boundary, which is the point.
const REACH_MEMORY: &[u8] = b"The on-call engineer for the Helios ground station is Marisol Vance.";

/// The App the `reach-inherit-principal` task runs. It declares NO tools — its steering
/// sets `reach: inherit_principal`, which REPLACES the (empty) declared wish with the
/// caller's whole resolvable ceiling. A tool firing under this App therefore fired
/// because reach widened the entry step's contract, not because the App asked for it.
///
/// The prompt NAMES the tool so that inheritance is the only way it can fire — tool
/// SELECTION under a wide menu is what the `tool` family measures, not this one. `guards`
/// pins the same budget every other family runs under, so families are compared on equal
/// terms rather than one silently inheriting a different default.
///
/// This task FOUND a real defect and is now its regression guard. It dead-lettered on
/// turn 0 on both engines — "the chain could not progress" — while `GetAppManifest`
/// cheerfully reported `reach_inherit` with 7 inherited tools. The cause was not reach
/// and not the model: the blueprint base every authored step is built from carried the
/// PURE demo recipe's 30 s inference budget, so an App's agentic turn ran on a quarter of
/// what the same model gets under `kx/recipes/react` and simply timed out. It presented
/// as nondeterministic (fine on a fast turn, dead on a slow one), which is why a
/// single green run would have "confirmed" any theory. Fixed in
/// `provision::served_blueprint_base`; the committed baseline now records the working
/// behaviour, so a regression to the demo budget fails this gate.
fn reach_app_envelope() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "kortecx.app/v1",
        "version": "1",
        "name": "kx-bench-reach",
        "blueprint": { "steps": [ { "kind": "model", "prompt":
            "Use your mcp-kv/get tool to look up the value stored under the key 'b', \
             then report that value exactly." } ] },
        "steering_config": {
            "tools": { "reach": "inherit_principal" },
            "guards": { "max_turns": 8, "max_tool_calls": 6 }
        }
    }))
    .expect("the reach App envelope encodes")
}

/// The connector name the HTTP fixture registers under. Tool ids are `<server>/<remote>`,
/// so the model sees `fleet/page` and `fleet/get`.
const BENCH_HTTP_SERVER: &str = "fleet";

/// The `failure` family's tools: one binary, registered once per failure mode. The names
/// are what the task instructions steer at, and the modes are what the bin does.
const FLAKY_MODES: [(&str, &str); 4] = [
    ("flaky-healthy", "healthy"),
    ("flaky-error", "error"),
    ("flaky-garbage", "malformed"),
    ("flaky-slow", "slow"),
];

/// How long `flaky-slow` hangs — comfortably past the per-Mote deadline below, so the
/// task exercises the deadline rather than racing it.
const FLAKY_SLOW_SLEEP_MS: &str = "600000";

/// The per-Mote tool deadline the suite runs under. **Default OFF in the runtime**, so
/// without this the hanging tool would not be cut off at all and the task would sit
/// against the 600 s settle budget instead of dead-lettering. A benchmark for a timeout
/// has to switch the timeout on.
const TOOL_DEADLINE_SECS: &str = "20";

/// Decoy connectors that make the tool menu a real menu. Named to sort AFTER every tool
/// an oracle depends on, deliberately: the auto-grant cap keeps a deterministic
/// `(id, version)` PREFIX, so whatever sorts last is what gets dropped — and the things
/// that must survive are the ones the suite's expectations name.
const DECOY_COUNT: usize = 8;

/// Locate the bench flaky bin: an explicit override, else the workspace target dir.
fn bench_flaky_bin() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_MCP_BENCH_FLAKY_PATH") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    for profile in ["debug", "release"] {
        let candidate = root.join(profile).join("kx-mcp-bench-flaky");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Register the `failure` family's tools and the decoy menu.
///
/// Returns false when the bin is absent, so the families SKIP rather than scoring a
/// build problem as a capability failure — the same posture the reach fixtures take.
async fn register_flaky_and_decoys(c: &mut KxGatewayClient<Channel>) -> bool {
    let Some(bin) = bench_flaky_bin() else {
        eprintln!(
            "eval-bench: bench flaky bin absent — build it (`cargo build -p kx-mcp`) or set \
             KX_MCP_BENCH_FLAKY_PATH; the `failure` and `menu` families will skip"
        );
        return false;
    };
    let bin = bin.to_string_lossy().to_string();

    for (name, mode) in FLAKY_MODES {
        let mut args = vec![mode.to_string()];
        if mode == "slow" {
            args.push(FLAKY_SLOW_SLEEP_MS.to_string());
        }
        let reg = c
            .register_mcp_server(proto::RegisterMcpServerRequest {
                server_name: name.to_string(),
                transport: "stdio".to_string(),
                endpoint: bin.clone(),
                args,
                tls_required: false,
                credential_ref: String::new(),
                session_mode: "stateless".to_string(),
            })
            .await;
        match reg {
            Ok(r) => {
                let r = r.into_inner();
                eprintln!("eval-bench: {name} registered ({} tool(s))", r.discovered);
            }
            Err(e) => {
                eprintln!("eval-bench: {name} did NOT register — {e}");
                return false;
            }
        }
    }

    // The decoys. Each is a working tool with a plausible description, so choosing
    // between them is a real decision rather than a formality — and each is a candidate
    // for the grant cap to drop, which is why they sort last.
    for i in 0..DECOY_COUNT {
        let name = format!("zz-decoy-{i:02}");
        let _ = c
            .register_mcp_server(proto::RegisterMcpServerRequest {
                server_name: name,
                transport: "stdio".to_string(),
                endpoint: bin.clone(),
                args: vec!["healthy".to_string()],
                tls_required: false,
                credential_ref: String::new(),
                session_mode: "stateless".to_string(),
            })
            .await;
    }
    eprintln!("eval-bench: {DECOY_COUNT} decoy connector(s) registered — the menu is a menu");
    true
}

/// Register the HTTP fixture as a dialed MCP connector, and prove the egress policy is
/// still a policy.
///
/// Two registrations, deliberately. The first is the one the suite needs. The second is a
/// loopback address the operator did NOT allowlist, and it must be REFUSED — because a
/// change that admits an explicitly-named internal host and a change that opens the
/// internal network look identical from the passing side. The counter-case is what tells
/// them apart, so it runs before anything is scored rather than living only in a unit
/// test of the vetting function.
async fn register_bench_http(
    c: &mut KxGatewayClient<Channel>,
    fleet: &common::bench_http::BenchHttpServer,
) -> bool {
    // The un-allowlisted sibling: same loopback interface, a port nobody named. Refused
    // on the host, so the port is immaterial — which is the point.
    let unlisted = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: "kx-bench-unlisted".to_string(),
            transport: "http".to_string(),
            endpoint: "http://10.0.0.7/mcp".to_string(),
            args: vec![],
            tls_required: false,
            credential_ref: String::new(),
            session_mode: "stateless".to_string(),
        })
        .await;
    assert!(
        unlisted.is_err(),
        "an internal host the operator never allowlisted must stay refused — otherwise \
         the admission gate is not a gate"
    );
    eprintln!("eval-bench: egress policy holds — an un-allowlisted internal host is refused");

    let reg = c
        .register_mcp_server(proto::RegisterMcpServerRequest {
            server_name: BENCH_HTTP_SERVER.to_string(),
            transport: "http".to_string(),
            endpoint: fleet.url(),
            args: vec![],
            tls_required: false,
            // The NAME of the env var the transport resolves at dispatch — never the
            // secret itself, which must not travel through a registration.
            credential_ref: common::bench_http::BENCH_HTTP_CRED_ENV.to_string(),
            session_mode: "stateless".to_string(),
        })
        .await;
    let reg = match reg {
        Ok(r) => r.into_inner(),
        Err(e) => {
            eprintln!("eval-bench: bench HTTP connector did NOT register — {e}");
            return false;
        }
    };
    eprintln!(
        "eval-bench: bench HTTP connector at {} — health {:?}, {} tool(s) discovered",
        fleet.url(),
        reg.health,
        reg.discovered
    );
    // Discovery already dialled the fixture, so the credential path has been exercised
    // before a single task runs. Assert it there: if the header never arrived, every
    // later call is a 401 and the family would score a model failure.
    let saw_auth = fleet.captured().iter().any(|r| r.auth_ok);
    assert!(
        saw_auth,
        "the Authorization header did not reach the fixture — the credential was named \
         but never injected, and the tasks below would measure that instead of the loop"
    );
    eprintln!("eval-bench: credential injection confirmed at the fixture");
    reg.discovered >= 1
}

/// Provision every `reach` fixture and HARD-assert each landed.
///
/// A fixture that silently failed to land is the worst failure mode this benchmark has:
/// the task still runs, the model still answers, and the oracle scores a 0 that reads as
/// a model incapability instead of an empty dataset. So each write is read back before a
/// single task is scored. Returns false when the serve cannot host the fixtures at all
/// (no embedder / memory disabled) — the caller then lets the family SKIP rather than
/// scoring it.
async fn provision_reach_fixtures(c: &mut KxGatewayClient<Channel>) -> bool {
    // (1) The retrieval corpus.
    let ingest = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: kx_gateway::eval_bench::BENCH_DATASET.to_string(),
            documents: REACH_CORPUS.iter().map(|d| doc(d)).collect(),
        })
        .await;
    if ingest.is_err() {
        eprintln!("eval-bench: reach fixtures unavailable — ingest failed (no embedder wired)");
        return false;
    }
    // The retrieval window the task itself gets. Read back at that width, not wider: a
    // fixture check that searched deeper than the task does would pass on a corpus the
    // task cannot actually solve.
    let hits = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: kx_gateway::eval_bench::BENCH_DATASET.to_string(),
            query_text: "Helios ground station eclipse-window callsign".to_string(),
            k: 8,
            ..Default::default()
        })
        .await
        .expect("query the freshly ingested bench dataset")
        .into_inner();
    assert!(
        !hits.hits.is_empty(),
        "the reach dataset must be searchable BEFORE scoring — an empty dataset scores 0 \
         and reads as a model failure"
    );
    // And the RIGHT document must be reachable. With sixty near-miss distractors — same
    // station with a different callsign, same callsign shape at a different station —
    // "some hit came back" stopped being evidence that the task is solvable. If the
    // load-bearing fact does not rank, the task measures the retriever's ceiling and
    // reports it as the model's; that is a fixture problem and it should fail here,
    // loudly, rather than downstream as a capability number.
    let retrieved_target = hits
        .hits
        .iter()
        .any(|h| String::from_utf8_lossy(&h.content).contains("ZEPHYR-77"));
    assert!(
        retrieved_target,
        "the grounding fact did not rank in the top {} of {} documents — the retrieval \
         task would score 0 for a reason that has nothing to do with the model. Top hits: {:?}",
        hits.hits.len(),
        REACH_CORPUS.len(),
        hits.hits
            .iter()
            .map(|h| String::from_utf8_lossy(&h.content)
                .chars()
                .take(60)
                .collect::<String>())
            .collect::<Vec<_>>()
    );
    eprintln!(
        "eval-bench: reach corpus = {} documents, grounding fact ranks in the top {}",
        REACH_CORPUS.len(),
        hits.hits.len()
    );

    // (2) The durable memory fact.
    if c.store_memory(proto::StoreMemoryRequest {
        content: REACH_MEMORY.to_vec(),
        embedding: Vec::new(),
        kind: proto::MemoryKind::Semantic as i32,
        namespace: String::new(),
    })
    .await
    .is_err()
    {
        eprintln!("eval-bench: reach fixtures unavailable — StoreMemory failed (KX_SERVE_MEMORY?)");
        return false;
    }
    let mems = c
        .list_memories(proto::ListMemoriesRequest {
            limit: Some(10),
            instance_id: None,
            namespace: String::new(),
            include_tombstoned: false,
        })
        .await
        .expect("list the freshly stored memory")
        .into_inner();
    assert!(
        mems.memories
            .iter()
            .any(|m| String::from_utf8_lossy(&m.content).contains("Marisol Vance")),
        "the recall fact must be readable BEFORE scoring — an empty memory scores 0 and \
         reads as a model failure"
    );

    // (3) The inherit-principal App.
    c.save_app(proto::SaveAppRequest {
        handle: kx_gateway::eval_bench::BENCH_REACH_APP_HANDLE.to_string(),
        envelope_json: reach_app_envelope(),
        source_digest: Vec::new(),
    })
    .await
    .expect("save the reach App");
    let manifest = c
        .get_app_manifest(proto::GetAppManifestRequest {
            handle: kx_gateway::eval_bench::BENCH_REACH_APP_HANDLE.to_string(),
        })
        .await
        .expect("read the reach App manifest")
        .into_inner();
    // The property the task exists to prove, asserted at the source: the App inherits.
    assert!(
        manifest.reach_inherit,
        "the reach App must report reach_inherit — without it the task would prove nothing"
    );
    assert!(
        manifest.tools.iter().any(|t| t.inherited),
        "every tool the reach App can reach must be INHERITED (it declared none itself)"
    );
    eprintln!(
        "eval-bench: reach fixtures ready — {} dataset hit(s), memory stored, App inherits {} tool(s)",
        hits.hits.len(),
        manifest.tools.iter().filter(|t| t.inherited).count()
    );
    true
}

/// The distinct `(tool_id, tool_version)` pairs the served runs actually committed — the
/// measure-first instrument for pinning `expected_tools` to the real grant-id form (which
/// is grant-shape-dependent, so it must be observed, not guessed).
async fn observed_tool_ids(c: &mut KxGatewayClient<Channel>) -> Vec<(String, String)> {
    let Ok(resp) = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(200),
            instance_id: None,
            step_salt: None,
        })
        .await
    else {
        return Vec::new();
    };
    let mut seen: Vec<(String, String)> = Vec::new();
    for t in resp.into_inner().turns {
        if t.branch == "tool" {
            let pair = (t.tool_id, t.tool_version);
            if !seen.contains(&pair) {
                seen.push(pair);
            }
        }
    }
    seen
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a Gemma GGUF (just fetch-gemma-model) or Ollama (KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b); opt in with --ignored"]
async fn bench_v1_oracle_scored_over_a_live_react_chain() {
    // Resolve the model: a GGUF (set the env so serve loads it) or the Ollama opt-in.
    if let Some(gguf) = serve_gguf() {
        std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
    } else if !ollama_opted_in() {
        eprintln!(
            "skipping: no model — `just fetch-gemma-model` (GGUF) or \
             `KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b`"
        );
        return;
    }
    std::env::set_var("KX_SERVE_AUTOGRANT", "1");

    // The one tool in this suite that is not a bundled mock. Started BEFORE the serve
    // because both the host allowlist and the credential are read at construction: the
    // fixture's port is only known now, and the allowlist is what turns the loopback
    // literal from refused into deliberately reachable.
    let fleet = common::bench_http::BenchHttpServer::start();
    std::env::set_var("KX_SERVE_TOOL_HOST_ALLOWLIST", fleet.host());
    std::env::set_var(
        common::bench_http::BENCH_HTTP_CRED_ENV,
        common::bench_http::BENCH_HTTP_BEARER,
    );
    // The per-Mote tool deadline is OFF by default, so a benchmark that includes a
    // hanging tool has to switch it on — otherwise `failure-timeout-deadletters` would
    // not measure a deadline, it would measure the settle budget several minutes later.
    std::env::set_var("KX_SERVE_TOOL_DEADLINE_SECS", TOOL_DEADLINE_SECS);

    let (engine, model) = engine_and_model();
    let capable = model.to_ascii_lowercase().contains("gemma");
    let env_label = env_label(&engine, &model);
    let git_sha = git_sha();

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // react-auto needs the model + the bundled echo/kv/calc capabilities. If react itself
    // didn't provision, the bundled bins are absent — skip (matches eval_real_model).
    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    if !recipes
        .recipes
        .iter()
        .any(|r| r.handle == REACT_RECIPE_HANDLE)
    {
        eprintln!(
            "skipping: {REACT_RECIPE_HANDLE} not provisioned — build the bundled tools \
             (`cargo build -p kx-mcp`)"
        );
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        clear_bench_env();
        clear_bench_env();
        return;
    }
    assert!(
        recipes
            .recipes
            .iter()
            .any(|r| r.handle == REACT_AUTO_RECIPE_HANDLE),
        "react-auto is provisioned alongside react"
    );

    // The real external tool, registered the way an operator registers one: over HTTP,
    // through admission vetting, with the credential named rather than embedded. HARD-
    // asserted rather than allowed to skip — a registration that quietly failed would
    // leave its tasks scoring 0, and a capability the runtime never offered the model
    // reads exactly like a model that could not use it.
    let http_ready = register_bench_http(&mut c, &fleet).await;
    assert!(
        http_ready,
        "the bench HTTP connector must register — without it the `http` family measures \
         the registration failure, not the runtime"
    );
    let flaky_ready = register_flaky_and_decoys(&mut c).await;

    // The `reach` family reads what the operator put there BEFORE the run — a dataset, a
    // durable memory, and an App that inherits the principal's ceiling. Provision them
    // first and read each back; a fixture that never landed would score 0 and read as a
    // model failure.
    let reach_ready = provision_reach_fixtures(&mut c).await;

    let corpus = kx_eval::load_bench_v1().expect("bench-v1 corpus loads");
    eprintln!(
        "eval-bench: scoring {} live task(s) on [{env_label}] (capable={capable}, \
         reach_fixtures={reach_ready}, http_tool={http_ready}, flaky_tools={flaky_ready}, \
         tool_deadline={TOOL_DEADLINE_SECS}s)",
        corpus.suite.tasks.len()
    );

    // THE WITNESS: drive every bench task on the served model + score its REAL output.
    let outcome = score_live_suite(
        &mut c,
        &corpus,
        env_label.clone(),
        git_sha.clone(),
        SETTLE_TIMEOUT,
    )
    .await
    .expect("score bench-v1 over live runs");
    let mut report = outcome.report.clone();
    for s in &outcome.skipped {
        eprintln!(
            "eval-bench: SKIPPED family {:?} — {} not provisioned ({} task(s): {})",
            s.family,
            s.missing_recipe,
            s.task_ids.len(),
            s.task_ids.join(", ")
        );
    }
    // A partial run is a partial measurement — say so loudly rather than let the number
    // read as full coverage.
    // `flaky_ready` counts here for the same reason `reach_ready` does: without the bin,
    // the `failure` and `menu` families ran against tools that were never registered, and
    // a baseline captured from that would ratchet the whole corpus against a subset while
    // reading as full coverage forever after.
    let complete = outcome.is_complete() && reach_ready && flaky_ready;
    if !complete {
        eprintln!("eval-bench: ⚠ INCOMPLETE COVERAGE — this run does NOT cover the whole corpus");
    }

    // Post-run identity check: the served model must match the label we recorded (a
    // capable run that silently fell back to a weak model would be a false record).
    let served = served_model_id(&mut c).await.unwrap_or_default();
    eprintln!("eval-bench: served model_id = {served:?}");
    if capable {
        assert!(
            served.to_ascii_lowercase().contains("gemma"),
            "identity check: asked for a Gemma model but the serve reports {served:?}"
        );
    }

    // The label that travels WITH the numbers into the committed baseline. Recorded from
    // the model the serve actually reports, not the one the operator asked for: those two
    // differ exactly when a run is worthless, and the whole point of a label is to be
    // right on the day it matters. Placed after the identity check above so a mismatch
    // fails before it can be written down.
    report.env = Some(kx_eval::BaselineEnv {
        engine: engine.clone(),
        model: if served.is_empty() {
            model.clone()
        } else {
            served.clone()
        },
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cores: u32::try_from(
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1),
        )
        .unwrap_or(1),
        task_count: u32::try_from(corpus.suite.tasks.len()).unwrap_or(u32::MAX),
        captured_unix_s: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        git_sha,
    });

    // The gates + the per-task oracle detail.
    eprintln!(
        "eval-bench report — suite '{}' (digest {}…)",
        report.suite_id,
        &report.suite_digest[..16]
    );
    if let Some(e) = &report.env {
        eprintln!(
            "  env: {} {} | {}/{} {} cores | {} tasks | sha {} | captured {}",
            e.engine,
            e.model,
            e.os,
            e.arch,
            e.cores,
            e.task_count,
            &e.git_sha[..e.git_sha.len().min(12)],
            e.captured_unix_s
        );
    }
    for g in &report.gates {
        eprintln!("  {:<22} {:>4} / 1000", g.id, g.per_mille);
    }
    // The measured, never-gated speed numbers. Printed BESIDE the gates and visibly
    // labelled as spikes, so nobody reads a latency as something the ratchet holds:
    // absolute milliseconds are a property of this machine, and only `model_time_share`
    // survives being compared across two of them.
    for s in &report.spikes {
        eprintln!(
            "  {:<22} {:>8.0} {} (spike — never gated)",
            s.id, s.value, s.unit
        );
    }
    if report
        .spikes
        .iter()
        .all(|s| s.id != "measured_tasks" || s.value == 0.0)
    {
        eprintln!(
            "eval-bench: ⚠ NO host timing was measured — the telemetry sidecar reported \
             nothing, so `model_time_share` is absent rather than 0"
        );
    }
    for t in &report.per_task {
        for s in &t.scores {
            if s.applicable {
                eprintln!("    {:<16} {:<22} {}", t.task_id, s.metric_id, s.detail);
            }
        }
    }
    // The ANSWER witness for a task that answered but failed its oracle. The trajectory
    // witness below covers runs that never reached an answer; this covers the other half,
    // which is just as much "a verdict without evidence": `answer missing oracle
    // substrings: ["384"]` tells you the substring was absent and NOTHING about what the
    // model actually said, so diagnosing it means re-running the suite by hand. Bounded,
    // because an answer is model-authored text of unbounded length.
    {
        let failed: std::collections::BTreeSet<&str> = report
            .per_task
            .iter()
            .filter(|t| {
                t.scores.iter().any(|s| {
                    s.metric_id == "task_success"
                        && matches!(s.value, kx_eval::ScoreValue::Gate { per_mille: 0 })
                })
            })
            .map(|t| t.task_id.as_str())
            .collect();
        for t in &outcome.transcripts {
            if !failed.contains(t.task_id.as_str()) {
                continue;
            }
            let Some(answer) = t.final_answer.as_deref() else {
                continue;
            };
            let shown: String = answer.chars().take(400).collect();
            let tail = if answer.chars().count() > 400 {
                "…"
            } else {
                ""
            };
            eprintln!(
                "eval-bench: ANSWER {} (oracle unsatisfied) → {shown:?}{tail}",
                t.task_id
            );
        }
    }
    // The TRAJECTORY witness for anything that did not answer. A per-task gate of 0 is a
    // verdict without evidence; this prints what the run actually did — which tool it
    // proposed, what the runtime refused and why, where it stopped — so a failure is
    // diagnosable from the run that produced it instead of re-run to be understood.
    for t in &outcome.transcripts {
        let terminal = t.terminal_branch();
        if terminal == kx_eval::Branch::Answer {
            continue;
        }
        eprintln!(
            "eval-bench: TRAJECTORY {} (terminal {terminal:?})",
            t.task_id
        );
        for turn in &t.turns {
            let tool = if turn.tool_id.is_empty() {
                String::new()
            } else {
                format!(" tool={}@{}", turn.tool_id, turn.tool_version)
            };
            let why = if turn.rejection_reason.is_empty() {
                String::new()
            } else {
                format!(" reason={:?}", turn.rejection_reason)
            };
            eprintln!("    turn {} {:?}{tool}{why}", turn.turn, turn.branch);
        }
        eprintln!(
            "    final_answer = {:?}",
            t.final_answer.as_deref().unwrap_or("<none>")
        );
    }

    // Measure-first: the raw committed tool-id form, to pin `expected_tools` against.
    let observed = observed_tool_ids(&mut c).await;
    eprintln!("eval-bench: observed committed tool ids = {observed:?}");

    // THE TOOL-CONTRACT ASSERTION, observed rather than inferred. `tool-contract-refusal`
    // instructs the model to use a tool no chain was ever granted. The oracle scores what
    // the model DID; this asserts the runtime invariant underneath it — the ungranted name
    // never entered any chain's admitted grant set, and never fired. Naming is not
    // granting, and here that is a measurement, not a claim.
    const UNGRANTED: &str = "admin-db/drop_table";
    assert!(
        !observed.iter().any(|(id, _)| id == UNGRANTED),
        "an UNGRANTED tool fired: {UNGRANTED} appears in the committed tool ids {observed:?}"
    );
    let admitted = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: Some(200),
            instance_id: None,
            step_salt: None,
        })
        .await
        .map(|r| {
            r.into_inner()
                .turns
                .into_iter()
                .flat_map(|t| t.granted_tools)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !admitted.iter().any(|g| g.contains(UNGRANTED)),
        "the ungranted {UNGRANTED} must never appear in a chain's admitted grants"
    );
    eprintln!(
        "eval-bench: tool contract holds — {UNGRANTED} absent from {} admitted grant entries",
        admitted.len()
    );

    // THE GRANT GUARD, and it is what makes every other number in this report mean
    // something. The auto-grant union caps the menu and keeps a deterministic prefix by
    // (id, version), so a tool whose id sorts late is silently dropped once enough tools
    // are registered — and the suite now registers a great many. A dropped oracle tool
    // produces exactly the same output as a model that declined to use it: the task
    // scores 0 and the table reads as a capability the model lacks. Assert instead that
    // every tool the corpus depends on was actually offered.
    {
        let granted: std::collections::BTreeSet<String> = admitted.iter().cloned().collect();
        let mut missing: Vec<String> = Vec::new();
        for t in &corpus.suite.tasks {
            for want in &t.expect.expected_tools {
                // A grant entry is rendered `<id>@<ver>`; match on the id it contains
                // rather than reconstructing the exact rendering.
                if !granted.iter().any(|g| g.contains(&want.tool_id)) {
                    missing.push(format!("{} (for {})", want.tool_id, t.id));
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "these tools an oracle depends on were never granted to the model — almost \
             certainly the auto-grant cap dropping late-sorting ids. Their tasks would \
             have scored 0 and read as model failures: {missing:?}"
        );
        eprintln!(
            "eval-bench: grant guard holds — every expected tool was offered ({} distinct \
             grant entries)",
            granted.len()
        );
    }

    // Persist the env-labelled trend record (the gitignored docs/benchmarks sink).
    std::fs::create_dir_all(benchmarks_dir()).unwrap();
    let trend = benchmarks_dir().join(format!("bench-v1.{engine}.json"));
    std::fs::write(&trend, report.to_json().unwrap()).unwrap();
    eprintln!("eval-bench: trend record → {}", trend.display());

    // Capture mode: write the committed per-engine baseline and stop (a deliberate
    // re-baseline, mirroring `kx-eval run --update-baseline`).
    if std::env::var("KX_BENCH_UPDATE_BASELINE").is_ok() {
        // The baseline is keyed by `suite_digest` — the WHOLE corpus. Capturing one that
        // silently omitted a family would ratchet the full corpus against a subset and
        // read as full coverage forever after. Refuse, loudly.
        assert!(
            complete,
            "refusing to capture a baseline from an INCOMPLETE run — the committed \
             baseline is keyed by the whole corpus digest, so a partial capture would \
             ratchet every later run against a subset. Provision the missing families \
             (hnsw for react-rag, KX_SERVE_MEMORY for react-memory) and re-run."
        );
        let path = baseline_path(&engine);
        let json = serde_json::to_string_pretty(&report.to_baseline()).unwrap();
        std::fs::write(&path, format!("{json}\n")).unwrap();
        eprintln!("eval-bench: baseline captured → {}", path.display());
        running.shutdown().await.unwrap();
        std::env::remove_var("KX_SERVE_AUTOGRANT");
        clear_bench_env();
        clear_bench_env();
        return;
    }

    // Ratchet: if a committed baseline exists, it is the fail-closed regression gate
    // (fails on corpus drift or any gate below baseline). Assert it only for a capable
    // model — the weak stand-in never gates a real number.
    let bpath = baseline_path(&engine);
    let task_success = report
        .gates
        .iter()
        .find(|g| g.id == "task_success")
        .map_or(0, |g| g.per_mille);
    if bpath.is_file() {
        let baseline: kx_eval::Baseline =
            serde_json::from_str(&std::fs::read_to_string(&bpath).unwrap()).unwrap();
        let cmp = kx_eval::compare_to_baseline(&report, &baseline, BASELINE_TOLERANCE)
            .unwrap_or_else(|e| {
                // The corpus changed under the committed baseline. This is the ratchet
                // working, not a failure: the measurement contract moved, so every number
                // captured against the old corpus is void until an operator deliberately
                // re-captures on BOTH engines. Say exactly that instead of a raw error.
                panic!(
                    "{e}\n\n  The bench corpus changed since this baseline was captured.\n  \
                     Re-capture BOTH engines deliberately:\n    \
                     KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma3:12b \
                     KX_BENCH_UPDATE_BASELINE=1 just eval-bench\n    \
                     ollama stop gemma3:12b   # GPU residency is a cross-engine singleton\n    \
                     KX_SERVE_MODEL_GGUF=<gemma-12b.gguf> KX_BENCH_UPDATE_BASELINE=1 just eval-bench"
                )
            });
        if cmp.ok {
            eprintln!(
                "eval-bench: PASS — all gates >= baseline {}",
                bpath.display()
            );
        } else {
            eprintln!(
                "eval-bench: {} regression(s) vs {}",
                cmp.regressions.len(),
                bpath.display()
            );
            for r in &cmp.regressions {
                eprintln!(
                    "  - {}: {} < baseline {}",
                    r.metric_id, r.current_per_mille, r.baseline_per_mille
                );
            }
            // Gate a capable model against the ratchet — but ONLY on a complete run. On
            // an incomplete one the missing family's tasks were never scored (or scored
            // against fixtures that never landed), so a "regression" would be blaming the
            // model for the serve's build. Loud, not fatal.
            if capable && complete {
                panic!("eval-bench regressed below the committed baseline");
            }
            if capable {
                eprintln!(
                    "eval-bench: regression NOT gated — coverage was incomplete, so these \
                     numbers indict the serve's provisioning, not the model"
                );
            }
        }
    } else {
        eprintln!(
            "eval-bench: no committed baseline at {} — record-only (capture with \
             KX_BENCH_UPDATE_BASELINE=1)",
            bpath.display()
        );
    }

    if capable && complete {
        assert!(
            task_success >= CAPABLE_TASK_SUCCESS_FLOOR,
            "capable-model task_success {task_success} < floor {CAPABLE_TASK_SUCCESS_FLOOR}"
        );
    } else if capable {
        eprintln!("eval-bench: RECORD-ONLY (incomplete coverage — the oracle floor is not gated)");
    } else {
        eprintln!(
            "eval-bench: RECORD-ONLY (weak stand-in {model:?} — the oracle floor is not gated)"
        );
    }

    running.shutdown().await.unwrap();
    std::env::remove_var("KX_SERVE_AUTOGRANT");
    clear_bench_env();
}
