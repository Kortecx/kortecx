//! `kx new <kind> <name> [--dir <parent>]` — offline scaffolders (no gateway):
//!
//! - `kx new skill <name>` — a `kortecx.skill/v1` pack (RC-SW1, D175): the three
//!   pack files with a template the author fills in, then the conformance + add +
//!   registry checklist.
//! - `kx new connector <name>` — a bundled MCP connector *sidecar* crate (RC-SW2,
//!   the D175 test-infra scaffolder): a self-contained stdio JSON-RPC 2.0 server
//!   (`initialize` → `tools/list` → `tools/call`) with ONE starter tool, the
//!   credential-by-reference (D81) discipline, an offline FAKE mode, and a
//!   conformance test — modelled on `integrations/kx-connector-discord` so a
//!   parallel session can stand a new integration up in minutes. It has **no
//!   `kx-*` runtime dependency** (an external process the runtime dials), so it
//!   cannot move the projection digest or perturb the frozen trio.
//!
//! What both deliberately do NOT do: contact a gateway, derive refs/hashes, run
//! conformance, edit `Cargo.toml` members / `registry/index.json` /
//! `feature-ledger.toml` — the emitted README carries the next-steps checklist.

use std::path::{Path, PathBuf};

use crate::client::next_value;
use crate::error::CliError;

/// The `new` subcommand: `skill` (RC-SW1) or `connector` (RC-SW2).
#[derive(Debug)]
pub enum NewSub {
    /// Scaffold a skill pack directory.
    Skill {
        /// The skill (and directory) name — `[a-z0-9._-]{1,64}`.
        name: String,
        /// Parent directory (default `.`); the pack lands at `<dir>/<name>/`.
        dir: PathBuf,
    },
    /// Scaffold a bundled MCP connector sidecar crate.
    Connector {
        /// The connector (provider) name — `[a-z0-9-]{1,48}`; the crate lands at
        /// `<dir>/kx-connector-<name>/`.
        name: String,
        /// Parent directory (default `integrations`, so the crate's dev-dep path
        /// to `kx-extension-sdk` resolves inside the workspace tree).
        dir: PathBuf,
    },
}

/// Parsed `new` arguments.
#[derive(Debug)]
pub struct NewArgs {
    /// The subcommand.
    pub sub: NewSub,
}

/// The scaffolded `skill.json` (pack form — no `instructions_ref`; the server
/// derives it at `kx skills add`). `__NAME__` is substituted.
const SKILL_JSON_TEMPLATE: &str = r#"{
  "schema": "kortecx.skill/v1",
  "name": "__NAME__",
  "version": "1",
  "description": "One sentence: what outcome this skill produces.",
  "tags": [],
  "tools": {}
}
"#;

const INSTRUCTIONS_TEMPLATE: &str = "# __NAME__

You are … (the role this skill gives the agent).

## Procedure

1. …the ordered steps; name each tool you expect to use and when.
2. …

## Boundaries

- …what this skill must never do (the wish set below enforces the hard line;
  write the soft lines here).

## Output contract

…the shape of the final answer the user should get.
";

const README_TEMPLATE: &str = "# __NAME__

A `kortecx.skill/v1` pack: declarative instructions + a tool grant-WISH set.
Attaching it grants nothing — at run the server intersects the wish against the
caller's grants and the live broker (`wish ∩ grants ∩ fireable`).

Fill in `skill.json` `tools` with the `(tool_id → version)` wishes, e.g.
`{\"retrieve\": \"1\", \"gmail/search\": \"1\"}` (a connector tool is
`<connection-name>/<tool>`), and write the instructions in `instructions.md`.

## Next steps

1. `just test-skill <this-dir>` — the declarative conformance gate (or
   `cargo run -p kx-extension-sdk --example skill_conformance -- <this-dir>`).
2. `kx skills add --dir <this-dir>` — add it to your serve's catalog.
3. `kx app new <app> --from-blueprint <bp.json> --skill __NAME__` — attach it.
4. Upstreaming in-tree? Add a `registry/index.json` entry + a
   `feature-ledger.toml` row (`just registry-check` verifies).
";

// ---------------------------------------------------------------------------
// connector scaffold templates. `__NAME__` = the kebab provider name;
// `__ENV__`  = its env-var form (uppercased, `-`→`_`), for the credential var.
// ---------------------------------------------------------------------------

const CONNECTOR_CARGO_TEMPLATE: &str = r#"# SPDX-License-Identifier: Apache-2.0
[package]
name         = "kx-connector-__NAME__"
version      = "0.1.0"
edition      = { workspace = true }
rust-version = { workspace = true }
license      = { workspace = true }
authors      = { workspace = true }
repository   = { workspace = true }
description  = "kortecx bundled __NAME__ MCP connector — a standalone MCP stdio server (initialize -> tools/list -> tools/call). Authenticates by-reference (D81): a credential is injected out-of-band BY NAME (KX___ENV___CREDENTIAL) and used INSIDE this process; the secret value never appears in a reply, a log, or the runtime. An external process the runtime DIALS via `kx connections add` — never linked into the gateway or the frozen trio (no kx-* dependency)."

[lib]
path = "src/lib.rs"

# The __NAME__ MCP connector binary — dialed via
# `kx connections add --command kx-connector-__NAME__ --credential-ref KX___ENV___CREDENTIAL`.
[[bin]]
name = "kx-connector-__NAME__"
path = "src/main.rs"

[dependencies]
serde      = { workspace = true }
serde_json = { workspace = true, features = ["raw_value"] }
ureq       = { workspace = true }
thiserror  = { workspace = true }

[dev-dependencies]
# The conformance harness (dials THIS bin through the real register_server path).
kx-extension-sdk = { path = "../../crates/kx-extension-sdk", version = "0.1.0" }

[lints]
workspace = true
"#;

const CONNECTOR_MAIN_TEMPLATE: &str = r#"// SPDX-License-Identifier: Apache-2.0
//! The `kx-connector-__NAME__` binary: a newline-delimited JSON-RPC 2.0 MCP server
//! over stdio. It builds its client from the environment (the injected credential,
//! D81) once at start, then answers one request per input line. The credential
//! value is never written to stdout, stderr, or a log.

use std::io::{BufRead, Write};

use kx_connector___NAMEID__::{handle_line, Client};

fn main() {
    let client = Client::from_env();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = handle_line(&line, &client);
        if writeln!(out, "{reply}").is_err() || out.flush().is_err() {
            break;
        }
    }
}
"#;

const CONNECTOR_LIB_TEMPLATE: &str = r##"// SPDX-License-Identifier: Apache-2.0
//! `kx-connector-__NAME__` — a bundled __NAME__ MCP connector (scaffold).
//!
//! A standalone Model Context Protocol server: newline-delimited JSON-RPC 2.0 over
//! stdio, speaking `initialize` -> `tools/list` -> `tools/call`. It ships ONE
//! starter tool (`ping`); replace it with your provider's real tools.
//!
//! ## Credential discipline (D81 — secret-by-reference)
//! The runtime resolves the connection's `credential_ref` NAME against the caller's
//! own secret store and injects the VALUE into this process's environment
//! ([`CREDENTIAL_ENV`]). Read it, authenticate to your provider INSIDE this process,
//! and NEVER place the value in a reply, a log line, or an error — so it never
//! reaches the runtime's journal, a `MoteId`, or a staged effect.
//!
//! ## Offline mode (tests / CI / conformance)
//! With [`FAKE_ENV`] set the connector runs fully offline with deterministic canned
//! responses (no network, no credential), so the MCP protocol + the
//! secret-never-echoed contract are gated without a live credential.
//!
//! This crate is an **external process** the runtime dials — no dependency on the
//! gateway, the journal, or the frozen trio, so building or running it cannot move
//! the projection digest or perturb the core. As it grows, split `Client` / the MCP
//! protocol / the tool catalog into modules (see `integrations/kx-connector-discord`
//! for the pattern).

#![allow(clippy::module_name_repetitions)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)
)]

use serde::Deserialize;
use serde_json::value::RawValue;

/// The env var NAME (D81) under which the runtime injects this connector's
/// credential VALUE out-of-band. The value never appears in a reply/log/error.
pub const CREDENTIAL_ENV: &str = "KX___ENV___CREDENTIAL";
/// Offline switch: canned deterministic responses (no network, no credential).
pub const FAKE_ENV: &str = "KX___ENV___FAKE";
/// The MCP protocol version advertised at `initialize`.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The __NAME__ client — holds the injected credential (never logged) + a fake flag.
pub struct Client {
    #[allow(dead_code)]
    credential: Option<String>,
    fake: bool,
}

impl Client {
    /// Build from the environment: the injected credential (by NAME, D81) + fake mode.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            credential: std::env::var(CREDENTIAL_ENV).ok(),
            fake: std::env::var(FAKE_ENV).is_ok(),
        }
    }

    /// A credential-free offline client for tests.
    #[must_use]
    pub fn fake() -> Self {
        Self {
            credential: None,
            fake: true,
        }
    }

    /// The starter `ping` tool: echoes a note back. Replace with your real provider
    /// calls. NEVER return the credential in any branch.
    fn ping(&self, note: &str) -> Result<String, String> {
        if self.fake {
            return Ok(format!(
                r#"{{"status":"ok","echo":{},"mode":"fake"}}"#,
                json_str(note)
            ));
        }
        // TODO(author): call your provider's REST API here (blocking `ureq`), using
        // `self.credential` for auth INSIDE this process. Return a JSON string; map
        // failures to `Err(reason)` (the reason must NEVER include the credential).
        Ok(format!(
            r#"{{"status":"ok","echo":{},"mode":"live"}}"#,
            json_str(note)
        ))
    }
}

/// Parse one newline-delimited JSON-RPC request and produce its reply line.
/// Fail-closed: an unparseable line yields a parse error (`-32700`).
#[must_use]
pub fn handle_line(line: &str, client: &Client) -> String {
    match serde_json::from_str::<Req>(line) {
        Ok(req) => handle(&req, client),
        Err(_) => error(0, -32700, "parse error"),
    }
}

#[derive(Deserialize)]
struct Req {
    #[serde(default)]
    id: u64,
    method: String,
    #[serde(default)]
    params: Option<Params>,
}

#[derive(Deserialize)]
struct Params {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<Box<RawValue>>,
}

fn handle(req: &Req, client: &Client) -> String {
    let id = req.id;
    match req.method.as_str() {
        "initialize" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":"{PROTOCOL_VERSION}","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"kx-connector-__NAME__","version":"1"}}}}}}"#
        ),
        "tools/list" => format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"tools":{CATALOG}}}}}"#
        ),
        "tools/call" => call(req, client),
        other => error(id, -32601, &format!("no such method: {other}")),
    }
}

/// The `tools/list` catalog. Add one object per tool your connector exposes.
const CATALOG: &str = r#"[{"name":"ping","description":"A starter tool: echoes a note back. Replace with your real tools.","inputSchema":{"type":"object","properties":{"note":{"type":"string"}},"required":[]}}]"#;

#[derive(Deserialize)]
struct PingArgs {
    #[serde(default)]
    note: Option<String>,
}

fn call(req: &Req, client: &Client) -> String {
    let id = req.id;
    let params = req.params.as_ref();
    let name = params.and_then(|p| p.name.as_deref()).unwrap_or_default();
    let args_raw = params
        .and_then(|p| p.arguments.as_ref())
        .map_or_else(|| "{}".to_string(), |a| a.get().to_string());
    match name {
        "ping" => match serde_json::from_str::<PingArgs>(&args_raw) {
            Ok(a) => match client.ping(a.note.as_deref().unwrap_or("ping")) {
                Ok(json) => result(id, &json),
                Err(e) => error(id, -32000, &e),
            },
            Err(e) => error(id, -32602, &e.to_string()),
        },
        other => error(id, -32602, &format!("no such tool: {other}")),
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// A fail-closed JSON-RPC error reply. Never carries a credential value.
#[must_use]
pub fn error(id: u64, code: i64, message: &str) -> String {
    let msg = serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".to_string());
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{msg}}}}}"#)
}

/// A JSON-RPC success reply wrapping a `result` object given as raw JSON text.
#[must_use]
pub fn result(id: u64, result_json: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result_json}}}"#)
}

#[cfg(test)]
mod tests {
    use super::{handle_line, Client, PROTOCOL_VERSION};

    #[test]
    fn initialize_lists_ping_and_calls_it() {
        let c = Client::fake();
        assert!(handle_line(r#"{"id":1,"method":"initialize"}"#, &c).contains(PROTOCOL_VERSION));
        assert!(handle_line(r#"{"id":2,"method":"tools/list"}"#, &c).contains("ping"));
        let reply = handle_line(
            r#"{"id":3,"method":"tools/call","params":{"name":"ping","arguments":{"note":"hi"}}}"#,
            &c,
        );
        assert!(reply.contains(r#""result""#) && reply.contains("hi"));
    }

    #[test]
    fn parse_error_unknown_method_and_unknown_tool_fail_closed() {
        let c = Client::fake();
        assert!(handle_line("not json", &c).contains("-32700"));
        assert!(handle_line(r#"{"id":1,"method":"frob"}"#, &c).contains("-32601"));
        assert!(
            handle_line(r#"{"id":1,"method":"tools/call","params":{"name":"nope"}}"#, &c)
                .contains("-32602")
        );
    }
}
"##;

const CONNECTOR_CONFORMANCE_TEMPLATE: &str = r#"// SPDX-License-Identifier: Apache-2.0
//! Conformance: the scaffolded connector passes the Extension Acceptance Gate
//! subset (out-of-process · warrant/SN-8 · secret-by-ref · on/off), driven OFFLINE
//! (`KX___ENV___FAKE`) so it needs no credentials and no network.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_extension_sdk::conformance::{run_conformance, ConnectorUnderTest};
use kx_extension_sdk::prelude::{SessionMode, TransportSpec};

#[test]
fn connector_passes_conformance_offline() {
    const SECRET: &str = "SEKRET-__ENV__-DEADBEEF-do-not-leak-0123456789";
    const CRED_VAR: &str = "KX___ENV___CREDENTIAL";

    std::env::set_var("KX___ENV___FAKE", "1");
    std::env::set_var(CRED_VAR, SECRET);

    let cut = ConnectorUnderTest {
        name: "__NAME__".into(),
        transport: TransportSpec::Stdio {
            command: env!("CARGO_BIN_EXE_kx-connector-__NAME__").to_string(),
            args: vec![],
        },
        credential_ref: Some(CRED_VAR.to_string()),
        session_mode: SessionMode::Stateless,
    };
    let report = run_conformance(&cut);

    std::env::remove_var(CRED_VAR);
    std::env::remove_var("KX___ENV___FAKE");

    assert!(report.reachable, "connector should be reachable: {report:#?}");
    assert!(report.discovered >= 1, "expected the ping tool: {report:#?}");
    assert!(report.passed(), "connector failed conformance: {report:#?}");
    for item in [3u8, 5, 7, 10] {
        assert!(
            report.checks.iter().any(|c| c.gate_item == item && c.passed),
            "gate item {item} missing or failed: {report:#?}"
        );
    }
}
"#;

const CONNECTOR_README_TEMPLATE: &str = "# kx-connector-__NAME__

A bundled __NAME__ MCP connector *sidecar*: a standalone stdio JSON-RPC 2.0 server
(`initialize` → `tools/list` → `tools/call`) with one starter tool. It has **no
`kx-*` runtime dependency** — an external process the runtime DIALS, so building or
running it cannot move the projection digest (`7d22d4bd…`) or touch the frozen trio.

Credentials are **by reference** (D81): the runtime injects your credential VALUE
into `KX___ENV___CREDENTIAL` out-of-band; this process reads it and authenticates
INSIDE itself. The value NEVER appears in a reply, a log, or an error.

## Next steps

1. Implement your tools in `src/lib.rs` (replace the `ping` starter): add a row to
   `CATALOG`, a decode struct, and a `Client` method; keep FAKE mode deterministic.
2. `cargo test -p kx-connector-__NAME__` — the unit + conformance tests
   (offline, `KX___ENV___FAKE`).
3. Wire it into the workspace: add `\"integrations/kx-connector-__NAME__\"` to the
   root `Cargo.toml` `members`, and (to upstream) a `registry/index.json` entry +
   a `feature-ledger.toml` row (`just registry-check` verifies).
4. Use it: `kx connections add --command kx-connector-__NAME__ --credential-ref \\
   KX___ENV___CREDENTIAL` then `kx connections fire __NAME__/ping --arg note=hi`,
   or attach it to an App and run via `RunApp` (`.with_connection(...)`).
";

/// Parse `new` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<NewArgs, CliError> {
    let kind = args
        .next()
        .ok_or_else(|| CliError::Usage("new requires a kind: skill | connector".into()))?;
    if kind != "skill" && kind != "connector" {
        return Err(CliError::Usage(format!(
            "unknown new kind {kind:?} (expected skill | connector)"
        )));
    }
    let default_dir = if kind == "connector" {
        "integrations"
    } else {
        "."
    };
    let mut name: Option<String> = None;
    let mut dir = PathBuf::from(default_dir);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--dir" => dir = PathBuf::from(next_value(&mut args, "--dir")?),
            other if other.starts_with("--") => {
                return Err(CliError::Usage(format!("unknown flag {other:?}")))
            }
            positional if name.is_none() => name = Some(positional.to_string()),
            extra => return Err(CliError::Usage(format!("unexpected argument {extra:?}"))),
        }
    }
    let name = name.ok_or_else(|| CliError::Usage(format!("new {kind} requires <name>")))?;
    let sub = if kind == "connector" {
        NewSub::Connector { name, dir }
    } else {
        NewSub::Skill { name, dir }
    };
    Ok(NewArgs { sub })
}

/// Execute `new` (offline).
pub fn execute(args: NewArgs) -> Result<(), CliError> {
    match args.sub {
        NewSub::Skill { name, dir } => execute_skill(&name, &dir),
        NewSub::Connector { name, dir } => execute_connector(&name, &dir),
    }
}

fn execute_skill(name: &str, dir: &Path) -> Result<(), CliError> {
    // The same grammar kx-skill enforces — fail here with the author-friendly
    // message instead of at add time.
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
    {
        return Err(CliError::Usage(format!(
            "skill name must be 1-64 chars of [a-z0-9._-], got {name:?}"
        )));
    }
    let pack = dir.join(name);
    ensure_empty_dir(&pack)?;
    for (file, template) in [
        ("skill.json", SKILL_JSON_TEMPLATE),
        ("instructions.md", INSTRUCTIONS_TEMPLATE),
        ("README.md", README_TEMPLATE),
    ] {
        write_file(&pack.join(file), &template.replace("__NAME__", name))?;
    }
    println!(
        "scaffolded skill pack {}\n  next: edit skill.json (the tool wishes) + instructions.md, \
         then `just test-skill {}` and `kx skills add --dir {}`",
        pack.display(),
        pack.display(),
        pack.display()
    );
    Ok(())
}

fn execute_connector(name: &str, dir: &Path) -> Result<(), CliError> {
    // A connector crate name is `kx-connector-<name>`; the provider name is a strict
    // kebab identifier so it is a valid crate-name suffix + a URL-path-safe tool
    // namespace.
    if name.is_empty()
        || name.len() > 48
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(CliError::Usage(format!(
            "connector name must be 1-48 chars of [a-z0-9-] (no leading/trailing '-'), got {name:?}"
        )));
    }
    let env = name.to_ascii_uppercase().replace('-', "_");
    let name_id = name.replace('-', "_");
    let subst = |t: &str| {
        t.replace("__NAMEID__", &name_id)
            .replace("__NAME__", name)
            .replace("__ENV__", &env)
    };
    let crate_dir = dir.join(format!("kx-connector-{name}"));
    ensure_empty_dir(&crate_dir)?;
    write_file(
        &crate_dir.join("Cargo.toml"),
        &subst(CONNECTOR_CARGO_TEMPLATE),
    )?;
    write_file(
        &crate_dir.join("src/main.rs"),
        &subst(CONNECTOR_MAIN_TEMPLATE),
    )?;
    write_file(
        &crate_dir.join("src/lib.rs"),
        &subst(CONNECTOR_LIB_TEMPLATE),
    )?;
    write_file(
        &crate_dir.join("tests/conformance.rs"),
        &subst(CONNECTOR_CONFORMANCE_TEMPLATE),
    )?;
    write_file(
        &crate_dir.join("README.md"),
        &subst(CONNECTOR_README_TEMPLATE),
    )?;
    println!(
        "scaffolded connector crate {}\n  next: implement your tools in src/lib.rs, run \
         `cargo test -p kx-connector-{}`, add it to the workspace `members`, then \
         `kx connections add --command kx-connector-{} --credential-ref KX_{}_CREDENTIAL`",
        crate_dir.display(),
        name,
        name,
        env
    );
    Ok(())
}

/// Fail-closed: never clobber an existing non-empty directory; create it otherwise.
fn ensure_empty_dir(path: &Path) -> Result<(), CliError> {
    if path.exists()
        && path
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(CliError::Usage(format!(
            "{} already exists and is not empty",
            path.display()
        )));
    }
    std::fs::create_dir_all(path).map_err(|e| CliError::Usage(format!("{}: {e}", path.display())))
}

/// Write `contents` to `path`, creating parent directories as needed.
fn write_file(path: &std::path::Path, contents: &str) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::Usage(format!("{}: {e}", parent.display())))?;
    }
    std::fs::write(path, contents).map_err(|e| CliError::Usage(format!("{}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_skill_with_a_dir() {
        let a = parse(
            ["skill", "triage", "--dir", "/tmp/x"]
                .iter()
                .map(|s| (*s).to_string()),
        )
        .unwrap();
        let NewSub::Skill { name, dir } = a.sub else {
            panic!("expected skill");
        };
        assert_eq!(name, "triage");
        assert_eq!(dir, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn parses_new_connector_defaulting_dir_to_integrations() {
        let a = parse(["connector", "slack"].iter().map(|s| (*s).to_string())).unwrap();
        let NewSub::Connector { name, dir } = a.sub else {
            panic!("expected connector");
        };
        assert_eq!(name, "slack");
        assert_eq!(dir, PathBuf::from("integrations"));
    }

    #[test]
    fn rejects_missing_name_bad_kind_and_unknown_flags() {
        let p = |parts: &[&str]| parse(parts.iter().map(|s| (*s).to_string()));
        assert!(p(&[]).is_err());
        assert!(p(&["gadget", "x"]).is_err(), "unknown kind");
        assert!(p(&["skill"]).is_err(), "needs a name");
        assert!(p(&["connector"]).is_err(), "needs a name");
        assert!(p(&["skill", "x", "--frob", "y"]).is_err());
    }

    #[test]
    fn the_emitted_skill_template_passes_pack_validation() {
        // Template-drift pin: what `kx new skill` emits must load as a valid pack.
        let tmp = tempfile::tempdir().unwrap();
        execute(
            parse(
                ["skill", "my-skill", "--dir", tmp.path().to_str().unwrap()]
                    .iter()
                    .map(|s| (*s).to_string()),
            )
            .unwrap(),
        )
        .unwrap();
        let pack = kx_skill::SkillPack::load_dir(&tmp.path().join("my-skill")).unwrap();
        assert_eq!(pack.manifest.name, "my-skill");
        assert!(pack.manifest.tools.is_empty(), "wishes start empty");
        assert!(pack.readme.is_some());
    }

    #[test]
    fn the_emitted_connector_crate_has_the_expected_files_and_substitutions() {
        let tmp = tempfile::tempdir().unwrap();
        execute_connector("my-thing", tmp.path()).unwrap();
        let crate_dir = tmp.path().join("kx-connector-my-thing");
        for f in [
            "Cargo.toml",
            "src/main.rs",
            "src/lib.rs",
            "tests/conformance.rs",
            "README.md",
        ] {
            assert!(crate_dir.join(f).exists(), "missing {f}");
        }
        let cargo = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name         = \"kx-connector-my-thing\""));
        assert!(
            cargo.contains("KX_MY_THING_CREDENTIAL"),
            "env var substituted"
        );
        assert!(
            !cargo.contains("__NAME__") && !cargo.contains("__ENV__"),
            "no placeholders left"
        );
        let lib = std::fs::read_to_string(crate_dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("KX_MY_THING_CREDENTIAL") && lib.contains("KX_MY_THING_FAKE"));
        assert!(!lib.contains("__NAME__") && !lib.contains("__NAMEID__"));
        let main = std::fs::read_to_string(crate_dir.join("src/main.rs")).unwrap();
        assert!(
            main.contains("kx_connector_my_thing::"),
            "crate ident substituted"
        );
    }

    #[test]
    fn refuses_a_non_empty_target_and_bad_names() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(execute_skill("UPPER", tmp.path()).is_err());
        assert!(execute_connector("Bad_Name", tmp.path()).is_err());
        assert!(execute_connector("-lead", tmp.path()).is_err());
        execute_skill("taken", tmp.path()).unwrap();
        assert!(
            execute_skill("taken", tmp.path()).is_err(),
            "never clobbers a non-empty pack"
        );
    }
}
