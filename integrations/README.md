<!-- SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0 -->
# `integrations/` — bundled app connectors

This directory holds **app-integration connectors**: standalone MCP-server processes
the runtime *dials* (via `kx connections add`) to let agents consume, generate, and
publish through external services (Gmail, Slack, Discord, Notion, …).

Each connector is a **self-contained leaf crate** with **no `kx-*` runtime
dependency** — it is an external process, never linked into the gateway, the
journal, or the frozen trio. Building or running a connector therefore cannot move
the projection digest (`7d22d4bd…`) or perturb the core. They live under
`integrations/` (not `crates/`) to keep the extension surface visibly separate from
the core library crates. They are workspace members (so `cargo`/`clippy`/`fmt`
cover them) and are registered in `shared-paths.toml` (`integrations/**` is shared)
and in `feature-ledger.toml`.

Authoring conventions follow the Connector/Extension SDK (D167 E0): build on the
`kx-extension-sdk` template (`crates/kx-extension-sdk/src/bin/reference_connector.rs`)
and gate with its `run_conformance` harness (Extension Acceptance Gate items
3/5/7/10: out-of-process · warrant/SN-8 · secret-by-ref · on/off).

## Connectors

| Crate | Provider | Tools | Status |
|---|---|---|---|
| `kx-connector-gmail` | Gmail | `search` · `read` · `draft` · `send` | connector shipped (PR-G0); core wiring pending (below) |
| `kx-connector-discord` | Discord | `send_message` · `read_channel` · `list_channels` | connector shipped; core wiring pending (below) |
| `kx-connector-slack` | Slack | `post_message` · `read_channel` · `search` · `list_channels` | connector shipped; core wiring pending (below) |
| `kx-connector-notion` | Notion | `search` · `read_page` · `create_page` · `append_block` | connector shipped; core wiring pending (below) |

### `kx-connector-gmail`

A newline-delimited JSON-RPC 2.0 stdio MCP server. Credential-by-reference (D81):
the OAuth credential is injected out-of-band as the env var `KX_GMAIL_CREDENTIAL`
(JSON `{"client_id","client_secret","refresh_token"}`); the connector does the
refresh→access-token exchange **inside its own process** and calls the Gmail REST
API. The secret value never appears in a reply, a log, or an error.

Register it against a running `kx serve`:

```sh
kx secrets set --name KX_GMAIL_CREDENTIAL \
  --value '{"client_id":"…","client_secret":"…","refresh_token":"…"}'
kx connections add --name gmail \
  --command kx-connector-gmail \
  --credential-ref KX_GMAIL_CREDENTIAL
# an agent granted gmail/* can now fire the tools:
kx connections fire --name gmail --tool search --args '{"query":"is:unread"}'
```

Offline/CI mode: set `KX_GMAIL_FAKE=1` for deterministic canned responses (no
network, no credential) — used by the unit tests and the conformance gate. A live
GR15/GR24 witness is `tests/live_smoke.rs` (`#[ignore]`; needs a real credential).

### `kx-connector-discord`

A newline-delimited JSON-RPC 2.0 stdio MCP server. Credential-by-reference (D81):
a Discord **bot token** is injected out-of-band as the env var
`KX_DISCORD_CREDENTIAL` (JSON `{"bot_token":"…"}`); the connector calls the Discord
REST API with an `Authorization: Bot <token>` header **inside its own process**. The
secret value never appears in a reply, a log, or an error. Channel/guild ids are
validated as bare snowflakes (digits only) before any request is built, so a path
separator can never smuggle into the URL.

Register it against a running `kx serve`:

```sh
kx secrets set --name KX_DISCORD_CREDENTIAL \
  --value '{"bot_token":"…"}'
kx connections add --name discord \
  --command kx-connector-discord \
  --credential-ref KX_DISCORD_CREDENTIAL
# an agent granted discord/* can now fire the tools:
kx connections fire --name discord --tool list_channels --args '{"guild_id":"…"}'
kx connections fire --name discord --tool send_message \
  --args '{"channel_id":"…","content":"hello from kortecx"}'
```

Offline/CI mode: set `KX_DISCORD_FAKE=1` for deterministic canned responses (no
network, no credential) — used by the unit tests and the conformance gate. A live
GR15/GR24 witness is `tests/live_smoke.rs` (`#[ignore]`; needs a real bot token +
`KX_DISCORD_TEST_GUILD_ID`).

### `kx-connector-slack`

A newline-delimited JSON-RPC 2.0 stdio MCP server. Credential-by-reference (D81):
a Slack **bot token** (`xoxb-…`) is injected out-of-band as the env var
`KX_SLACK_CREDENTIAL` (JSON `{"bot_token":"…"}`); the connector calls the Slack Web
API with an `Authorization: Bearer <token>` header **inside its own process**. The
secret value never appears in a reply, a log, or an error.

Register it against a running `kx serve`:

```sh
kx secrets set --name KX_SLACK_CREDENTIAL \
  --value '{"bot_token":"xoxb-…"}'
kx connections add --name slack \
  --command kx-connector-slack \
  --credential-ref KX_SLACK_CREDENTIAL
# an agent granted slack/* can now fire the tools:
kx connections fire --name slack --tool list_channels --args '{}'
kx connections fire --name slack --tool post_message \
  --args '{"channel":"C0123ABCD","text":"hello from kortecx"}'
```

Offline/CI mode: set `KX_SLACK_FAKE=1` for deterministic canned responses (no
network, no credential) — used by the unit tests and the conformance gate. A live
GR15 witness is `tests/live_smoke.rs` (`#[ignore]`; needs a real bot token in
`KX_SLACK_CREDENTIAL`).

### `kx-connector-notion`

A newline-delimited JSON-RPC 2.0 stdio MCP server. Credential-by-reference (D81):
a Notion **integration token** is injected out-of-band as the env var
`KX_NOTION_CREDENTIAL` (JSON `{"token":"…"}`); the connector calls the Notion REST
API with an `Authorization: Bearer <token>` header plus the required
`Notion-Version` header **inside its own process**. The secret value never appears
in a reply, a log, or an error.

Register it against a running `kx serve`:

```sh
kx secrets set --name KX_NOTION_CREDENTIAL \
  --value '{"token":"secret_…"}'
kx connections add --name notion \
  --command kx-connector-notion \
  --credential-ref KX_NOTION_CREDENTIAL
# an agent granted notion/* can now fire the tools:
kx connections fire --name notion --tool search --args '{"query":"roadmap"}'
kx connections fire --name notion --tool append_block \
  --args '{"page_id":"…","text":"a note from kortecx"}'
```

Offline/CI mode: set `KX_NOTION_FAKE=1` for deterministic canned responses (no
network, no credential) — used by the unit tests and the conformance gate. A live
GR15 witness is `tests/live_smoke.rs` (`#[ignore]`; needs a real integration token
in `KX_NOTION_CREDENTIAL` + `KX_NOTION_TEST_PAGE_ID`).

### App-pointer run — build an App that USES the connection (G1 + G2, landed)

An App can now carry a *pointer* to a connection and dial it inside its agentic loop.
Connect Gmail first-class, then run an App that references it:

```sh
# G1: one-click connect (fills command + credential-ref from the curated catalog)
kx connections add --provider gmail          # ≡ --name gmail --command kx-connector-gmail
                                              #    --credential-ref KX_GMAIL_CREDENTIAL
kx secrets set --name KX_GMAIL_CREDENTIAL --value '{"client_id":"…","client_secret":"…","refresh_token":"…"}'

# author an App that REFERENCES the connection + scopes its credential (Py SDK)
python - <<'PY'
import kortecx as kx
app = (kx.app("gmail-agent")
       .blueprint(kx.flow().agent("Search my unread Gmail and summarise it.", tools=["gmail/search"]))
       .with_gmail()          # declare the connection + add KX_GMAIL_CREDENTIAL to secret_scope
       .steer(max_turns=4, max_tool_calls=2))
app.save(handle="apps/local/gmail-agent")
PY

# G2: run it SERVER-SIDE (RunApp) — honors references.connections + guards.secret_scope
kx app run apps/local/gmail-agent --wait
```

At run time `RunApp` reads the validated stored envelope, resolves the `ConnectionRef`
against the **caller's own** registered connection (by credential-ref name), and sets
the run warrant's `SecretScope::AllowList` to the App's `guards.secret_scope` — so the
broker precheck lets the agent dial the credentialed connector. Because the pointer is
a bare *name*, a shared App resolves **each operator's own** credentials. Set
`KX_GMAIL_FAKE=1` for a deterministic, network-free witness (a real MCP subprocess with
canned upstream responses). See `docs/site/docs/apps.md` for the full chaining guide.

---

## Core wiring — status

**G1 (first-class Connect Gmail) + G2 (App-pointer → run resolution) are LANDED.** An
operator connects Gmail in one step and an App that references it dials the connector
inside its agentic loop (above). Both are off-journal + additive (digest `7d22d4bd…`
invariant, frozen trio untouched). **G3 (cross-instance import) is the remaining piece.**

### G1 — first-class Gmail Integration across UI / CLI / SDK ✅
- Curated "Gmail" provider: `kx connections add --provider gmail`, a "Connect Gmail"
  prefill chip in `ConnectionsPanel`, and `.with_gmail()` on the Py/TS App builders —
  all over the **existing** `RegisterMcpServer` + `PutSecret` RPCs (no new proto). A
  small static provider catalog (id → command + credential-ref) mirrored across
  CLI/SDK/UI; Discord/Slack/Notion clone the Gmail row when curated.

### G2 — App-pointer → run resolution ✅
- A server-side **`RunApp(handle, args)`** RPC + a host `AppAuthor` seam
  (`crates/kx-gateway/src/app_run.rs`): the gateway reads the validated stored envelope
  (server-owned — no client-forged references, SN-8), lowers its blueprint through the
  shared FFI-free `kx-blueprint` crate (byte-identical to the client path), resolves
  `references.connections` against the caller's own registry (missing → a clear
  `failed_precondition("missing integration: <name>")`), and sets the tool-firing
  warrant's `SecretScope::AllowList` to the App's `guards.secret_scope`
  (`crates/kx-warrant/src/secret.rs`). CLI/Py/TS `run_app` prefer `RunApp` and fall
  back to the legacy `GetApp` → `SubmitWorkflow` on an older server. Off-journal,
  additive, digest `7d22d4bd…` invariant (the new path defaults to `SecretScope::None`
  ⇒ existing runs byte-identical; recovery replays the journaled `warrant_ref`).

### G3 — cross-instance App import + rebind (the "share to others" story)
- **What:** the deferred cross-instance import entrypoint (today
  `gateway.proto:1033,2170`: "NO cross-instance import entrypoint (deferred)"). Build
  it so an App exported from one instance and imported on another rebinds to **that**
  instance's connections/secrets by name — the App carries no authority (it "grants
  nothing; references intersect with the importer's own grants ∩ the step warrant").
- **Security negatives to close first (POC-4 deferral):** (1) envelope-authority
  leakage, (2) credential-namespace collision (the importer's secret must win),
  (3) warrant/principal spoofing (validate the importer's warrant covers the App's
  requested grants, server-side, before bind).
- **How:** additive import RPC + `crates/kx-gateway/src/apps.rs` (the party-scoped
  `apps.db` catalog) + rebind validation. Off-journal, additive (no journal fact).
- **Effort:** L. **Blast radius:** medium + a real security surface.

### Out of scope for OSS (Cloud)
Per-party credential isolation on a **single shared instance** (multiple users, each
with their own credentials resolved per-caller): connections/secrets are
operator-global today (`connections.db` PK is `name`; the OS keychain is
per-machine), and D129 / D170.b allocate multi-tenant secrets + a KMS/HSM vault +
the connector marketplace to **Cloud**. Cross-instance sharing (each user runs their
own `kx serve`, per G3) is the OSS path.
