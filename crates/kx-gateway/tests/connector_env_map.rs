//! A REAL third-party MCP server, configured with the environment it actually needs, and
//! FIRED — asserting the tool's own observable effect rather than that a row exists.
//!
//! The server under test is `@modelcontextprotocol/server-gitlab`, pinned in the
//! `real-connector` fixture and installed offline by `just test-connector-real`. It was
//! chosen by probing eleven published MCP servers for what their SHIPPED code actually
//! reads from the environment; it is the one that reads TWO variables and makes both
//! load-bearing:
//!
//! ```text
//! dist/index.js:17-21   GITLAB_PERSONAL_ACCESS_TOKEN absent -> process.exit(1)
//! dist/index.js:24      `${GITLAB_API_URL}/projects/...`   -> decides WHERE every call goes
//! dist/index.js:29      Authorization: Bearer ${TOKEN}     -> decides WHO it calls as
//! ```
//!
//! So neither variable alone configures it, which is exactly the capability this file
//! exists to prove — and `one_variable_cannot_configure_a_two_variable_server` asserts
//! that gap directly, using the single `credential_ref` that was the only mechanism
//! available before the environment map. Without that arm, a passing map test could be
//! passing for reasons that have nothing to do with the map.
//!
//! Everything is loopback: `GITLAB_API_URL` points at `common::gitlab_stub`, which answers
//! `401` unless the `Authorization` header carries the exact expected token, and otherwise
//! returns a schema-complete GitLab project. That makes BOTH variables discriminable from
//! the tool's own result, with no network and no real GitLab account:
//!
//! - wrong token, right URL -> the server reports `Unauthorized`
//! - right token, other URL -> the OTHER stub's marker comes back
//! - both right             -> a payload only this stub could have produced
//!
//! No secret VALUE is ever registered: the map's right-hand side is a credential REF name,
//! resolved in the transport at spawn and dropped.

#![cfg(feature = "mcp-gateway")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use common::gitlab_stub::GitLabStub;
use kx_gateway::start;
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The variables the SERVER reads. Fixed by the third party, not by us.
const VAR_TOKEN: &str = "GITLAB_PERSONAL_ACCESS_TOKEN";
const VAR_API_URL: &str = "GITLAB_API_URL";

const GOOD_TOKEN: &str = "glpat-w3-expected-0123456789";
const WRONG_TOKEN: &str = "glpat-w3-rejected-9876543210";

/// The credential ref NAMES for one test, all carrying that test's own suffix.
///
/// Per-test rather than shared constants because these resolve out of the PROCESS
/// environment and the tests in this binary run concurrently: a shared `..._API_URL_REF`
/// would be rewritten by whichever test seeded last, silently pointing one test's server
/// at another test's stub. The ref names are also deliberately unlike the variables they
/// feed — if both namespaces used the same string, the target-name plumbing could be doing
/// nothing and every assertion here would still pass.
struct Refs {
    token: String,
    api_url: String,
    wrong_token: String,
}

impl Refs {
    /// Seed this test's refs into the process environment, where the OSS `EnvSecretStore`
    /// resolves a ref by name — the same resolution `kx secrets set` feeds.
    fn seed(suffix: &str, api_url: &str) -> Self {
        let refs = Self {
            token: format!("KX_W3_TOKEN_REF_{suffix}"),
            api_url: format!("KX_W3_API_URL_REF_{suffix}"),
            wrong_token: format!("KX_W3_WRONG_TOKEN_REF_{suffix}"),
        };
        std::env::set_var(&refs.token, GOOD_TOKEN);
        std::env::set_var(&refs.wrong_token, WRONG_TOKEN);
        std::env::set_var(&refs.api_url, api_url);
        refs
    }

    /// The well-formed two-entry map: the token and the API base URL, each by reference.
    fn map(&self) -> Vec<(&str, &str)> {
        vec![
            (VAR_TOKEN, self.token.as_str()),
            (VAR_API_URL, self.api_url.as_str()),
        ]
    }
}

/// Everything this test binary logs, at TRACE, in one buffer.
///
/// Deliberately process-wide and maximally verbose: a leak scan should read the NOISIEST
/// output the runtime can produce, because that is where a value would surface. Installed
/// once, so every test in this binary contributes and the assertion below covers all of
/// them rather than one arm.
static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

#[derive(Clone)]
struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut g) = self.0.lock() {
            g.extend_from_slice(buf);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Install the capturing subscriber (idempotent) and return the shared buffer.
fn captured_logs() -> Arc<Mutex<Vec<u8>>> {
    LOGS.get_or_init(|| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = CaptureWriter(buf.clone());
        // `set_global_default` can fail if something else already installed one; the
        // capture is then empty, and `a_planted_value_is_visible_to_the_log_scanner`
        // below FAILS rather than letting an empty buffer read as perfect hygiene.
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(writer)
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .finish(),
        );
        buf
    })
    .clone()
}

fn log_text(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&buf.lock().expect("log buffer").clone()).into_owned()
}

/// The pinned third-party server, installed offline by `just test-connector-real`.
fn gitlab_server_bin() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../kx-extension-sdk/tests/fixtures/real-connector/node_modules/.bin/mcp-server-gitlab",
    );
    p.is_file().then_some(p)
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

/// The environment map rides `RegisterMcpServerWithEnv`, not `RegisterMcpServer`: the plain
/// request is a `ControlPreview` arm, and an environment is not something a
/// natural-language proposal may carry.
fn register(
    server_name: &str,
    endpoint: &str,
    credential_ref: &str,
    env: Vec<(&str, &str)>,
) -> proto::RegisterMcpServerEnvRequest {
    proto::RegisterMcpServerEnvRequest {
        base: Some(proto::RegisterMcpServerRequest {
            server_name: server_name.to_string(),
            transport: "stdio".to_string(),
            endpoint: endpoint.to_string(),
            args: vec![],
            tls_required: false,
            credential_ref: credential_ref.to_string(),
            session_mode: "stateless".to_string(),
        }),
        env: env
            .into_iter()
            .map(|(name, credential_ref)| proto::McpEnvEntry {
                name: name.to_string(),
                credential_ref: credential_ref.to_string(),
            })
            .collect(),
    }
}

/// ★ THE PROOF. A two-entry environment map configures a real third-party server, and the
/// tool it exposes fires with an effect only the stub could have produced.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn an_env_map_configures_and_fires_a_real_third_party_server() {
    let Some(bin) = gitlab_server_bin() else {
        panic!(
            "the pinned third-party MCP server is missing — run `just test-connector-real` \
             (npm ci in crates/kx-extension-sdk/tests/fixtures/real-connector). This is a \
             FAILURE rather than a skip: a proof that silently does not run is not a proof."
        );
    };
    let marker = "kortecx/w3-env-map-proof";
    let stub = GitLabStub::start(GOOD_TOKEN, marker);
    let refs = Refs::seed("PROOF", &stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let reg = c
        .register_mcp_server_with_env(register("gitlab", &bin.to_string_lossy(), "", refs.map()))
        .await
        .expect("RegisterMcpServer reaches the gateway")
        .into_inner();
    assert_eq!(
        reg.health, "connected",
        "a server needing TWO variables dials cleanly once both are supplied"
    );
    assert!(
        reg.discovered > 0,
        "its tools are discovered: {}",
        reg.discovered
    );

    let before = stub.hits();
    let fired = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "gitlab".to_string(),
            remote_name: "search_repositories".to_string(),
            args_json: r#"{"search":"kortecx"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();

    assert!(fired.ok, "the third-party tool fired: {}", fired.error);
    // The observable effect, on BOTH variables at once: the payload came back (so the API
    // URL variable routed the call here) and the stub only serves it to the exact bearer
    // token (so the token variable arrived with the right value).
    assert!(
        fired.result_json.contains(marker),
        "the result carries the stub's own project — proof the call reached it and was \
         authorized. got: {}",
        fired.result_json
    );
    // The count the server folded out of the stub's `X-Total` header. The tool result is
    // JSON-in-JSON, so the inner quotes arrive escaped — match on the value, not on a
    // quoting shape that depends on how many times the payload has been wrapped.
    assert!(
        fired.result_json.contains("count") && fired.result_json.contains(": 7"),
        "the count folded from the stub's X-Total header is present: {}",
        fired.result_json
    );
    assert!(
        stub.hits() > before,
        "the stub observed the request (hits {} -> {})",
        before,
        stub.hits()
    );

    // No secret VALUE anywhere on the governance surface — only variable NAMES.
    let listed = c
        .list_mcp_servers(proto::ListMcpServersRequest::default())
        .await
        .expect("ListMcpServers reaches the gateway")
        .into_inner();
    let row = listed
        .servers
        .iter()
        .find(|s| s.server_name == "gitlab")
        .expect("the registered server is listed");
    assert_eq!(
        row.env_names,
        vec![VAR_TOKEN.to_string(), VAR_API_URL.to_string()],
        "the governance view names the variables, in declaration order"
    );
    let listed_json = format!("{listed:?}");
    assert!(
        !listed_json.contains(GOOD_TOKEN),
        "no secret VALUE on the list surface"
    );
    assert!(
        !listed_json.contains(&refs.token),
        "not even the REF the variable maps to — names only"
    );

    running.shutdown().await.unwrap();
}

/// ★ THE CONTROL that makes the proof mean something: the SAME server, configured the only
/// way that was possible before the environment map — a single `credential_ref`, injected
/// under its own name — cannot be made to work.
///
/// The single credential is spent on the token, so `GITLAB_API_URL` is unset and the server
/// addresses `https://gitlab.com` instead of the stub. The assertion is on the stub: it is
/// never reached. Nothing here touches the network — the request either arrives at our
/// listener or the tool fails, and both outcomes are visible without leaving the machine.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn one_variable_cannot_configure_a_two_variable_server() {
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-should-not-be-reached");
    let _refs = Refs::seed("GAP", &stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // The pre-map mechanism: ONE ref, and the child sees it under the ref's own name. That
    // name has to be what the server reads, so the ref is literally named for the variable.
    std::env::set_var(VAR_TOKEN, GOOD_TOKEN);
    let reg = c
        .register_mcp_server_with_env(register(
            "gitlab1",
            &bin.to_string_lossy(),
            VAR_TOKEN,
            vec![],
        ))
        .await
        .expect("RegisterMcpServer reaches the gateway")
        .into_inner();
    assert_eq!(
        reg.health, "connected",
        "one variable is enough to START the server — which is exactly why the gap was \
         invisible: the connection looks healthy and only firing reveals it is misdirected"
    );

    let before = stub.hits();
    let fired = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "gitlab1".to_string(),
            remote_name: "search_repositories".to_string(),
            args_json: r#"{"search":"kortecx"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();

    assert_eq!(
        stub.hits(),
        before,
        "THE GAP: with only one variable configurable the server cannot be pointed at this \
         endpoint at all, so the call never arrives. hits stayed at {before}"
    );
    assert!(
        !fired.result_json.contains("w3-should-not-be-reached"),
        "and nothing from this stub can appear in the result: {}",
        fired.result_json
    );

    running.shutdown().await.unwrap();
}

/// The token entry is load-bearing, held to ONE varied variable: the same map, the same
/// URL, a ref resolving to a token the stub rejects. The refusal must name the REASON.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn a_wrong_credential_ref_in_the_map_is_refused_with_its_reason() {
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-env-map-proof");
    let refs = Refs::seed("WRONGCRED", &stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // ONE variable differs from the passing arm: the token's ref.
    c.register_mcp_server_with_env(register(
        "gitlabx",
        &bin.to_string_lossy(),
        "",
        vec![
            (VAR_TOKEN, refs.wrong_token.as_str()),
            (VAR_API_URL, refs.api_url.as_str()),
        ],
    ))
    .await
    .expect("RegisterMcpServer reaches the gateway");

    let before = stub.hits();
    let fired = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "gitlabx".to_string(),
            remote_name: "search_repositories".to_string(),
            args_json: r#"{"search":"kortecx"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();

    // The call DID arrive — so the URL entry still worked and this is a token failure,
    // not a routing failure. Distinguishing those is the whole point of asserting a reason.
    assert!(
        stub.hits() > before,
        "the request still reached the stub (the URL entry is unaffected)"
    );
    let detail = format!("{} {}", fired.error, fired.result_json);
    assert!(
        detail.contains("Unauthorized"),
        "the refusal names WHY — the server's own rejection of that token, not a generic \
         failure. got: {detail}"
    );

    running.shutdown().await.unwrap();
}

/// An entry naming a ref that resolves to nothing REFUSES the spawn, rather than starting
/// the child without it. A server that boots minus one variable fails later, in its own
/// vocabulary, describing a credential the runtime never sent.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn an_unresolvable_ref_refuses_rather_than_spawning_without_it() {
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-env-map-proof");
    let refs = Refs::seed("UNRESOLVABLE", &stub.url());
    std::env::remove_var("KX_W3_NO_SUCH_SECRET_REF");

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let reg = c
        .register_mcp_server_with_env(register(
            "gitlabu",
            &bin.to_string_lossy(),
            "",
            vec![
                (VAR_TOKEN, refs.token.as_str()),
                (VAR_API_URL, "KX_W3_NO_SUCH_SECRET_REF"),
            ],
        ))
        .await
        .expect("RegisterMcpServer reaches the gateway")
        .into_inner();

    // Registration is deliberately not fatal on a dial failure (the row persists so an
    // operator can fix the secret and re-test) — but the dial must NOT have succeeded.
    assert_ne!(
        reg.health, "connected",
        "an unresolvable ref refuses the spawn instead of dialing a half-configured server"
    );

    // The ACCEPTING control, one variable changed back: the same registration with a ref
    // that does resolve connects. Without this, the assertion above would also pass if
    // registration were broken for some unrelated reason.
    let ok = c
        .register_mcp_server_with_env(register("gitlabu2", &bin.to_string_lossy(), "", refs.map()))
        .await
        .expect("RegisterMcpServer reaches the gateway")
        .into_inner();
    assert_eq!(
        ok.health, "connected",
        "the same shape with a RESOLVABLE ref connects — so the refusal above was about \
         the ref, not about registration being broken"
    );

    running.shutdown().await.unwrap();
}

/// A malformed map is refused at admission with a reason, and each refusal has an
/// accepting sibling differing in exactly one thing.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn a_malformed_env_map_is_refused_at_admission() {
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-env-map-proof");
    let refs = Refs::seed("MALFORMED", &stub.url());
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // (a) the same variable twice — silently letting the last one win would make which
    // secret reached the server depend on declaration order.
    let dup = c
        .register_mcp_server_with_env(register(
            "dup",
            &bin.to_string_lossy(),
            "",
            vec![
                (VAR_TOKEN, refs.token.as_str()),
                (VAR_TOKEN, refs.wrong_token.as_str()),
            ],
        ))
        .await;
    let err = dup.expect_err("a duplicate variable is refused");
    assert!(
        err.message().contains("declared twice"),
        "the refusal names the duplicate: {}",
        err.message()
    );

    // (b) an entry with no ref — a variable declared with nothing to resolve it.
    let empty = c
        .register_mcp_server_with_env(register(
            "noref",
            &bin.to_string_lossy(),
            "",
            vec![(VAR_TOKEN, "")],
        ))
        .await;
    let err = empty.expect_err("an entry with no credential ref is refused");
    assert!(
        err.message().contains("credential ref"),
        "the refusal explains the by-reference rule: {}",
        err.message()
    );

    // (c) a variable colliding with the legacy single credential's own name.
    let collide = c
        .register_mcp_server_with_env(register(
            "collide",
            &bin.to_string_lossy(),
            VAR_TOKEN,
            vec![(VAR_TOKEN, refs.token.as_str())],
        ))
        .await;
    let err = collide.expect_err("a collision with credential_ref is refused");
    assert!(
        err.message().contains("declared twice"),
        "the refusal names the collision: {}",
        err.message()
    );

    // The ACCEPTING control: the same call with a well-formed map registers. Each refusal
    // above differs from this by exactly one thing.
    let ok = c
        .register_mcp_server_with_env(register(
            "wellformed",
            &bin.to_string_lossy(),
            "",
            refs.map(),
        ))
        .await
        .expect("a well-formed map registers")
        .into_inner();
    assert_eq!(ok.health, "connected");

    running.shutdown().await.unwrap();
}

/// ★ THE LOG LEG of the hygiene sweep: no environment VALUE reaches the runtime's logs.
///
/// The sink sweep in `kx-mcp` covers the journal, the staged bytes, the `MoteId`, the
/// effect payload and `Debug`. Logs are the remaining place a value could surface, and they
/// are the one an operator is most likely to paste into an issue. This drives a REAL
/// register-and-fire with a known value behind the map, then reads everything the process
/// logged at TRACE.
///
/// The scanner is proved FIRST, on a planted value it must find — an absence assertion over
/// a buffer that turned out to be empty is the exact shape of a scan that cannot fail.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn no_environment_value_reaches_the_logs() {
    let logs = captured_logs();
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-log-scan");
    let refs = Refs::seed("LOGSCAN", &stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.register_mcp_server_with_env(register(
        "gitlablog",
        &bin.to_string_lossy(),
        "",
        refs.map(),
    ))
    .await
    .expect("RegisterMcpServer reaches the gateway");

    let fired = c
        .call_mcp_tool(proto::CallMcpToolRequest {
            server_name: "gitlablog".to_string(),
            remote_name: "search_repositories".to_string(),
            args_json: r#"{"search":"kortecx"}"#.to_string(),
        })
        .await
        .expect("CallMcpTool reaches the gateway")
        .into_inner();
    assert!(
        fired.ok,
        "the tool fired, so values really were in play: {}",
        fired.error
    );

    // ⚠ THE POSITIVE CONTROL. Plant a value through the SAME writer the runtime logs
    // through, and require the scan to find it. If the subscriber never installed, or the
    // buffer is empty, or the read is wrong, this fails HERE — before any absence is
    // trusted.
    const PLANTED: &str = "PLANTED-LOG-CANARY-w3-0123456789";
    {
        let mut w = CaptureWriter(logs.clone());
        writeln!(w, "control: {PLANTED}").expect("write to the capture");
    }
    let text = log_text(&logs);
    assert!(
        text.contains(PLANTED),
        "the log scanner can see a value that IS there — without this the assertions \
         below would pass over an empty buffer"
    );
    assert!(
        text.len() > PLANTED.len() + 32,
        "the buffer holds the runtime's own output too, not just the control ({} bytes)",
        text.len()
    );

    // …and now the absences, over everything the process logged at TRACE.
    for value in [GOOD_TOKEN, WRONG_TOKEN, stub.url().as_str()] {
        assert!(
            !text.contains(value),
            "an environment VALUE reached the logs: {value}"
        );
    }
    // The NAMES are not secrets and may legitimately appear; the refs must not carry
    // their values alongside them.
    for pair in [
        format!("{VAR_TOKEN}={GOOD_TOKEN}"),
        format!("{}={GOOD_TOKEN}", refs.token),
    ] {
        assert!(
            !text.contains(&pair),
            "a name/value pair reached the logs: {pair}"
        );
    }

    running.shutdown().await.unwrap();
}

/// The post-registration lifecycle RPCs against a server that really answers.
///
/// Its sibling in `mcp_admin_rpcs` covers the same four RPCs with no connector at all — the
/// empty list and the reason-asserting refusals — and gates every PR because it depends on
/// nothing. What only a REAL server can show is the affirmative half: health folded from an
/// actual dial, a re-discovery that finds namespaced tools, and a deregistration that removes
/// a server that was genuinely there. That is what this adds, and why it lives here.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the pinned third-party MCP fixture — run `just test-connector-real`"]
async fn the_lifecycle_rpcs_answer_for_a_server_that_really_dials() {
    let Some(bin) = gitlab_server_bin() else {
        panic!("the pinned third-party MCP server is missing — run `just test-connector-real`");
    };
    let stub = GitLabStub::start(GOOD_TOKEN, "kortecx/w3-lifecycle");
    let refs = Refs::seed("LIFECYCLE", &stub.url());

    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    c.register_mcp_server_with_env(register("glc", &bin.to_string_lossy(), "", refs.map()))
        .await
        .expect("register the third-party connector");

    // ── ListMcpServers: exactly the row this test made, with health folded from a real dial.
    let listed = c
        .list_mcp_servers(proto::ListMcpServersRequest::default())
        .await
        .expect("ListMcpServers answers")
        .into_inner();
    assert_eq!(listed.servers.len(), 1, "exactly the connector just added");
    let row = &listed.servers[0];
    assert_eq!(row.server_name, "glc");
    assert_eq!(row.transport, "stdio");
    assert_eq!(row.health, "connected", "health is folded from a real dial");
    assert!(
        !row.connection_id.is_empty(),
        "the id is server-derived, never client-supplied"
    );

    // ── TestMcpServer: reachable, because it genuinely is.
    let ok = c
        .test_mcp_server(proto::TestMcpServerRequest {
            server_name: "glc".to_string(),
        })
        .await
        .expect("TestMcpServer answers")
        .into_inner();
    assert!(ok.reachable, "the connector is reachable: {}", ok.detail);

    // ── DiscoverServerTools: re-dials and returns the inventory, namespaced by its server.
    let found = c
        .discover_server_tools(proto::DiscoverServerToolsRequest {
            server_name: "glc".to_string(),
        })
        .await
        .expect("DiscoverServerTools answers")
        .into_inner();
    assert!(
        found.discovered > 0,
        "re-discovery finds the connector's tools: {}",
        found.discovered
    );
    assert!(
        found.tools.iter().all(|t| t.tool_name.starts_with("glc/")),
        "every discovered tool is namespaced by the server it came from: {:?}",
        found.tools.iter().map(|t| &t.tool_name).collect::<Vec<_>>()
    );

    // ── DeregisterMcpServer: removes a server that was really there…
    let removed = c
        .deregister_mcp_server(proto::DeregisterMcpServerRequest {
            server_name: "glc".to_string(),
        })
        .await
        .expect("DeregisterMcpServer answers")
        .into_inner();
    assert!(removed.removed, "the registered server is removed");
    let emptied = c
        .list_mcp_servers(proto::ListMcpServersRequest::default())
        .await
        .expect("ListMcpServers answers")
        .into_inner();
    assert!(emptied.servers.is_empty(), "…and it is gone from the list");

    // …and the lifecycle is genuinely over: testing it now refuses.
    let gone = c
        .test_mcp_server(proto::TestMcpServerRequest {
            server_name: "glc".to_string(),
        })
        .await
        .expect_err("a deregistered server is no longer testable");
    assert_eq!(gone.code(), tonic::Code::NotFound);

    running.shutdown().await.unwrap();
}
