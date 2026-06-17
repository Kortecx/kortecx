---
id: tools
title: Tools
sidebar_label: Tools
description: The Kortecx tools registry, the fs-list built-in, the ReAct tool loop, and the OSS/Cloud line.
---

# Tools

Kortecx agents call real **tools** inside the live ReAct loop — every tool turn
committed as a durable fact, exactly-once, replay-re-read. There are two surfaces:

- **The durable registry** (`DiscoverTools` / `RegisterTool` / `DeregisterTool`) —
  the governance inventory: what is registered, with what provenance, status, and
  egress authority. This is the source of truth for *what tools exist*.
- **Advisory discovery** (`kx tools list` / `kx tools score`) — a display-only
  ranking to help *choose* a tool. A score never authorizes anything (SN-8).

> **Authorization is always the runtime's, never a score or a registration.** A
> tool fires only under a **server-issued warrant**, re-verified by the broker at
> every call. Registering a tool grants no authority; the `tool_id` is
> server-derived (a client can never name or forge it). See
> [Security → model proposes, runtime enforces](./security.md#model-proposes-runtime-enforces).

## The registry (`DiscoverTools` / `RegisterTool` / `DeregisterTool`)

The registry is a durable, off-journal sidecar (`tools.db`) — rebuildable, never a
projection-digest input. The OSS built-ins (`fs-read@1`, `fs-write@1`,
`text-summarize@1`) are re-seeded on every open and cannot be deregistered.

### List the inventory

```bash
kx tools discover --limit 50
```

```python
from kortecx import KxClient

client = KxClient("http://localhost:50150", token="…")
page = client.discover_tools(limit=50)
for t in page.tools:
    print(t.tool_name, t.tool_version, t.kind, t.registration_status)
```

```typescript
import { KxClient } from "@kortecx/sdk/web";

const client = new KxClient("http://localhost:50150", { token: "…" });
const page = await client.discoverTools({ limit: 50 });
```

### Register a declarative external MCP tool

`RegisterTool` records a *single declarative* tool and its **SSRF-vetted** egress
host. The host is checked at admission (deny-by-default — internal / loopback /
link-local / metadata endpoints are refused); an optional operator allowlist is
`KX_SERVE_TOOL_HOST_ALLOWLIST`. This path records the tool; to **dial** a whole
external MCP server and auto-discover its tools, use
[Connections](#connections--the-external-mcp-gateway) (`kx connections add`).

```bash
kx tools register --name web-search --version 1 \
  --server-host mcp.example.com:443 \
  --description "search the web" --param q:str --param k:int
```

```python
tool_id = client.register_tool(
    name="web-search",
    version="1",
    server_host="mcp.example.com:443",
    description="search the web",
)
```

The returned `tool_id` is server-derived. A registered tool is **declared**, not
yet **fireable**: a tool fires only when this serve actually registers its
capability on the broker (today that is the bundled `mcp-echo@1` and `fs-list@1`).

### Deregister

```bash
kx tools deregister --name web-search --version 1
```

Built-ins are refused (the call returns `removed = false`).

## The `fs-list` built-in

`fs-list@1` is the first real host-side tool — a **read-only** directory lister,
gated behind the operator flag `KX_SERVE_FS_ROOT` (default-OFF, so the default
serve is byte-identical without it). When set, the runtime grants the ReAct loop a
read-only `fs_scope` of exactly that root and seeds the `kx/recipes/react-fs`
recipe; the model can then list files and reason over the result, committed as a
durable Observation.

```bash
KX_SERVE_FS_ROOT=/path/to/readable/dir kx serve --features inference
```

`fs-list` never reads file *contents* — it returns names only, confined to the
granted mount (canonicalized; no traversal out).

## Reviewing what agents produce

Every committed tool output (and every model turn) is captured in the **Data Lab →
Agent Outputs** lens — the Morphic action exhaust, newest-first, previewed in the
multi-modal viewer (a tool's JSON in Monaco, a model's answer as text, an image
inline). ReAct turns are badged by turn and branch. This is view-only in OSS;
cross-run analytics, dashboards, and synthesis are part of Kortecx Cloud.

## The ReAct tool loop

The model proposes a tool call; the runtime parses it through one fail-closed
authority gate (grant-checked against the step warrant, size-capped), validates
the arguments against the tool's typed `inputSchema`, and dispatches via the
capability broker. A prompt-injected call to an **ungranted** tool never fires.
See the [Quickstart agent loop](./quickstart.md#run-the-agent-loop).

## Authoring a `tool()` step

Beyond the ReAct loop (where the *model* picks tools), you can author a **`tool()`
step** into a DAG to fire a single registered tool deterministically — a discovered
external MCP tool, a `RegisterTool`'d declarative tool, or the bundled `fs-list`.
The server resolves the tool in its live registry and builds the per-step warrant
from the tool's **declared** capability scope (you never supply a warrant — the
client-`tool_grants` boundary stays refused, SN-8). The authored arguments lower to
one canonical-JSON object the coordinator validates against the tool's typed schema
fail-closed at every lease (so a crash re-derives byte-identical args).

```python
from kortecx import chain, model, tool

# A model plans a query, then a discovered tool runs it (a DATA edge feeds the result).
c = chain("plan > search", tasks={
    "plan": model("kx-serve:qwen3-4b-q4_k_m", "Plan a web search for the user's question."),
    "search": tool("search/web-search", "1", q="kortecx runtime"),
})
run = client.run_chain(c, wait=True)
```

```typescript
import { chain, task } from "@kortecx/sdk/web";
const c = chain("plan > search", { tasks: {
  plan: task.model("kx-serve:qwen3-4b-q4_k_m", "Plan a web search."),
  search: task.tool("search/web-search", "1", { q: "kortecx runtime" }),
} });
```

```bash
# kx blueprint run --file dag.json, where a step is:
#   { "kind": "tool", "tool_contract": { "search/web-search": "1" }, "args": { "q": "kortecx" } }
kx blueprint run --file dag.json --wait
```

In the **console builder** (`/blueprints/new`), add a **+ Tool** node, pick a
registered tool from the live registry (the same set `DiscoverTools` shows), and
edit its **Args (JSON)** — the server resolves + warrants it on submit. The
`q="…"`-style args lower **byte-identically across Python, TypeScript, Rust (CLI),
and the UI** (the `tests/golden/chains` parity gate).

## OSS / Cloud line

Kortecx OSS is the **secure gateway to external MCP** — the runtime is the
production-grade authority that connects to external MCP servers (which host the
tools). **No arbitrary code runs in the runtime.** In-runtime sandboxed
script/skill execution, and OAuth / hosted-marketplace credential connections, are
Cloud / future capabilities.

## Connections — the external MCP gateway

The runtime **dials external MCP servers** (stdio + HTTP, including Py/TS-SDK-
exposed gateways), discovers their tools, and registers each into the same durable
registry — namespaced `<server>/<remote>` so tools from different servers never
collide. Connections live in an off-journal, rebuildable `connections.db` sidecar
(never a `MoteId`/digest input); the `connection_id` is server-derived (SN-8).

```bash
# A local stdio MCP server:
kx connections add --name local --command /usr/local/bin/my-mcp-server --arg --stdio
# A remote HTTP MCP gateway (e.g. one you exposed from the Python/TS SDK):
kx connections add --name search --url https://mcp.example.com/rpc --tls-required \
  --credential-ref MCP_TOKEN
kx connections list
kx connections test --name search
kx connections discover --name search   # re-dial + re-discover its tools
kx connections remove --name search
```

```python
res = client.register_mcp_server(name="search", transport="http",
                                 endpoint="https://mcp.example.com/rpc",
                                 tls_required=True, credential_ref="MCP_TOKEN")
print(res.discovered, res.health)
for s in client.list_mcp_servers().servers:
    print(s.server_name, s.health, s.tool_count)
```

```typescript
await client.registerMcpServer({ name: "search", transport: "http",
  endpoint: "https://mcp.example.com/rpc", tlsRequired: true, credentialRef: "MCP_TOKEN" });
const { servers } = await client.listMcpServers();
```

The console's **Tools → Connections** panel is the live UI for this (add / test /
re-discover / remove, with per-server health). Manage it from whichever surface
fits your workflow.

### Security (the live untrusted-egress surface)

- **Two-gate egress.** The host is SSRF-vetted at **admission** (deny-by-default)
  AND again at **dial time** on the *resolved* address (DNS-rebind defense —
  loopback / private / link-local / `169.254.169.254` / CGNAT / IPv6-ULA refused).
- **Per-server rate-limit.** A token bucket per server bounds dial bursts.
- **Warrant-gated egress.** A discovered tool's `net_scope` is egress to *only*
  its origin server's host; the broker re-checks `request.net_scope ⊆ warrant` at
  every call. A prompt-injected call to an ungranted tool never fires (SN-8).
- **Secret-less credentials (D81).** A connection stores the credential **ref name
  only** (an env var / vault key); the secret value is read transiently at dial and
  never journaled / staged / shown to the model. A URL must not embed credentials
  in its userinfo (`user:pass@host` is refused at admission) — use `credential_ref`.
- **Registration is host-trusted.** A `credential_ref` names a process env var (the
  OSS `EnvSecretStore`), and a stdio connection names a program to spawn — both
  assume the operator registering the server is trusted with the serve host's
  environment + execution. Run `kx serve` as a principal whose env holds only the
  secrets you intend agents' tools to use; the Cloud `SecretStore` adds vault-scoped,
  multi-tenant credentials.

### What's still coming

OAuth / device-flow setup and a hosted credential marketplace are a **Cloud**
capability (the OSS console shows them as an honest-disabled affordance). The
[`tool()` chains-node](#authoring-a-tool-step) is now live (call a discovered tool
from an authored DAG, across Python / TypeScript / CLI / the console builder). The
autonomous ReAct loop's **auto-grant** of dialed tools (the model auto-picking from
the live dialed set) + **parallel tool fan-out** (N concurrent tool calls per turn)
land in the next batch — they share the coordinator's args-from-params
durable-execution path. (Branched write-back over the content-addressed store is the
D155 post-RC epic.)
