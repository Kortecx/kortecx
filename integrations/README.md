<!-- SPDX-License-Identifier: Apache-2.0 -->
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

---

## Core wiring still needed (the "make integrations first-class" roadmap)

The connector above is complete and dialable **today** — an operator can register it
and an agent granted its tools can call Gmail. What is **not yet wired in the core**
is the ergonomic, by-pointer app experience: a first-class "Gmail" entry in the
Integrations UI, and Apps that carry a *pointer* to the integration and resolve the
caller's own credentials at run time. These are **core changes** (gateway /
provision / UI / proto) taken up in their own sessions **after RC4c-2b**, each
off-journal + additive (digest `7d22d4bd…` invariant, frozen trio untouched).

### G1 — first-class Gmail Integration across UI / CLI / SDK
- **What:** a curated "Gmail" provider in the Integrations section ("Connect Gmail"
  → store the OAuth secret → register the connection), instead of the generic
  "add MCP server" form. Cross-surface: UI + `kx` + Py/TS SDK + a docs page.
- **How:** pure client-side curation over the **existing** RPCs — no new proto.
  Reuses `RegisterMcpServer` + `PutSecret`, `ui/src/components/tools/ConnectionsPanel.tsx`
  + `ui/src/kx/use-connections.ts`, and `crates/kx-cli/src/verbs/{connections,secrets}.rs`.
- **Effort:** S–M. **Blast radius:** low; off-journal; shared UI files ⇒ serialize on
  the shared lane.

### G2 — App-pointer → run resolution (the load-bearing gap)
- **What:** make an App actually *use* its integration pointer. Today at run time
  only the App **blueprint** is submitted; the envelope's
  `references.connections` and `guards.secret_scope` are dropped
  (`crates/kx-gateway/tests/app_live_serve.rs` submits `blueprint`, not `references`;
  `RecipeBinder::bind` in `crates/kx-gateway/src/provision.rs` has no app-references
  parameter). Wire it so that, at bind/run, the App's `ConnectionRef`s resolve the
  **caller's own** registered connection by name and the App's `secret_scope` names
  are injected into the run warrant's `SecretScope::AllowList`
  (`crates/kx-warrant/src/secret.rs`) so the broker precheck gates the Gmail secret.
- **Why it matters:** this is what makes "build an App that uses the Gmail pointer to
  consume/generate/publish" fire — and, because the pointer is a bare *name*, what
  makes a shared App resolve **each user's own** credentials.
- **How:** `crates/kx-gateway/src/provision.rs` (`RecipeBinder::bind`) + a
  gateway-core seam (`crates/kx-gateway-core/src/mcp_gateway_admin.rs`) + possibly one
  additive field on the app-run path. Off-journal, additive; the coordinator is
  editable (not frozen) but on the RC lane, so serialize.
- **Effort:** M. **Blast radius:** medium (touches the bind path; digest-invariant).

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
