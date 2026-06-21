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

### When a tool call fails

A refused proposal does not kill the run — it settles `rejected` and the model
re-prompts over the reason, bounded by the budget (see
[Agents → Graceful tool-call recovery](./agent-runner.md#graceful-tool-call-recovery)).
`kx react list --instance <id>` (or `ReactTurn.rejection_reason` in the SDKs)
shows why each turn was refused:

| What you see | What to do |
| --- | --- |
| `not granted to this run` | Grant the tool to the recipe/agent, or expect the model to pick a granted one. |
| `do not match its inputSchema` | The model's argument keys/types are wrong — the menu now shows an `Example:`; check the tool's declared `inputSchema`. |
| `could not be decoded` / `malformed` | The model emitted a non-tool or broken proposal — usually self-corrects on the next turn. |
| chain `dead_lettered` after several `rejected` turns | The budget (`max_turns` / `max_tool_calls`) was exhausted without a usable answer — raise the caps or simplify the task/schema. |

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

## Local function tools (`@kx.tool` / `localTool`)

Turn a plain function into a real, governed tool. The SDK exposes your decorated
functions as a **local stdio MCP server**, and the runtime **dials it through the
same external-MCP gateway** above — so a local function is just another dialed MCP
tool the runtime fires under a server-built warrant (SN-8). **No new runtime
substrate**, no proto change.

```python
import kortecx as kx

@kx.tool
def add(a: int, b: int) -> int:
    "Add two integers."
    return a + b

# Deterministic — fire it as one node (works today, even model-free):
print(kx.flow().tool(add, a=2, b=2).run().text)

# Steered — let a model decide (the react-auto loop; needs KX_SERVE_AUTOGRANT=1):
print(kx.Agent("Do the math.", tools=[add], dynamic=True).run("what is 2+2?").text)
```

```typescript
import { localTool, flow, Agent } from "@kortecx/sdk/node";

// TS types are erased at runtime, so the param schema is explicit:
const add = localTool({
  name: "add",
  params: { a: "integer", b: "integer" },
  run: ({ a, b }) => (a as number) + (b as number),
});

await flow().tool(add, { a: 2, b: 2 }).run({ client: kx });
await new Agent("Do the math.", { tools: [add], dynamic: true }).run("2+2", { client: kx });
```

**Schema from the signature.** Python derives the MCP `inputSchema` from your type
hints — `int` → integer, `str` → string, `bool` → boolean, `Literal[...]` / a
string `Enum` → an exact-match enum; a parameter with no default is required.
TypeScript declares the schema explicitly (`params`). **Floats are not type-gated**
by the runtime (pass an `int` where you can); nested objects fall back to a JSON
string.

**Three firing lanes (be honest about what runs today):**

| Lane | How | Status |
| --- | --- | --- |
| **Deterministic** | `flow().tool(fn, **args)` — one tool, fixed args | ✅ today (even model-free) — the reliable path for one fixed call |
| **Steered / dynamic** | `Agent(tools=[fn], dynamic=True)` → `kx/recipes/react-auto` (the model chooses) | ✅ the model picks tools turn by turn and fires them (needs a served model + `KX_SERVE_AUTOGRANT=1`) — a dialed tool's namespaced `&lt;server&gt;/&lt;name&gt;` name resolves from the model's bare/leaf call |
| **Frozen / deterministic-agentic** | `Agent(tools=[fn])` — a fixed tool set, replayable bounded loop | ✅ fires the granted SET in a bounded reason→tool→observe loop — **no** `KX_SERVE_AUTOGRANT` needed (the step grants its OWN exact tools) |

All three lanes fire today. The SN-8 authority gate stays exact: a model's bare/leaf or version-less name resolves to a **unique** granted tool (`&lt;server&gt;/&lt;name&gt;` → the grant), and ambiguity or an unknown name is refused fail-closed — the model never widens its grants.

**Dev-scoped & co-located.** The runtime *spawns* the stdio tool-server subprocess,
so this is the **Node / local-Python** SDK on the **same machine** as `kx serve`:
the interpreter, your tool module, and the runtime are co-located. Registering a
local tool is the same host-trusted operation as `kx connections add --command`
(see [Connections](#connections--the-external-mcp-gateway)) — V2b adds no new
attack surface. In Cloud the bridge is governed.

**Re-import contract.** The runtime spawns `python -m kortecx._toolserver`
(Node: `node _toolserver.js`) which re-loads your module to recover the functions.
Define your tools at **module level** and guard any `.run()` calls under
`if __name__ == "__main__":` (the loader runs the module under a non-`__main__`
name, so a guarded main block never re-executes).

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
# A server that needs a live session (browser automation, a DB transaction):
kx connections add --name browser --command /usr/local/bin/browser-mcp --stateful
kx connections list
kx connections test --name search
kx connections discover --name search   # re-dial + re-discover its tools
kx connections remove --name search
```

```python
res = client.register_mcp_server(name="search", transport="http",
                                 endpoint="https://mcp.example.com/rpc",
                                 tls_required=True, credential_ref="MCP_TOKEN",
                                 session_mode="stateless")  # the default
print(res.discovered, res.health)
for s in client.list_mcp_servers().servers:
    print(s.server_name, s.health, s.tool_count, s.session_mode)
```

```typescript
await client.registerMcpServer({ name: "search", transport: "http",
  endpoint: "https://mcp.example.com/rpc", tlsRequired: true, credentialRef: "MCP_TOKEN",
  sessionMode: "stateless" });
const { servers } = await client.listMcpServers();
```

The console's **Tools → Connections** panel is the live UI for this (add / test /
re-discover / remove, with per-server health). Manage it from whichever surface
fits your workflow.

### Session mode (stateless-first)

Each connection has a **firing posture**, chosen at registration (`--session-mode`,
`session_mode=`, the **Session** chip in the console; default **stateless**):

- **Stateless** (the default) — every tool call is a self-contained, single-shot
  session: dial → `initialize` → `tools/call` → close. This is the best fit for
  idempotent read tools (search, retrieval, file listing) and for remote servers
  behind a round-robin load balancer, and it aligns with the runtime's durable
  model — a stateless call is a *content-addressed fact* that recovers from a crash
  with no session store. If a server needs to keep state across calls, the MCP
  pattern is to mint an explicit handle (a `basket_id`, a `browser_id`) from a tool
  and pass it back as an ordinary argument — that handle rides in the committed
  Mote, so it survives recovery with no sticky session.
- **Stateful** — the runtime keeps **one long-lived session** and reuses it across
  calls, amortizing the handshake. Use it only for servers that genuinely require a
  live session (browser automation, a database transaction, a stateful sandbox) or
  for chatty same-server traffic. The session is re-opened automatically after any
  transport fault.

### MCP protocol interoperability

The client speaks the **MCP `2026-07-28`** revision: it advertises that version on
`initialize` and (for HTTP) sends the `MCP-Protocol-Version` routing header on every
request so a server behind a plain load balancer can route without inspecting the
body. It **captures the version the server negotiates** in its reply and proceeds
either way — so the runtime interoperates with **both** older (`2025-06-18`) and
RC (`2026-07-28`) servers. An older server simply negotiates down; nothing is
refused on a version mismatch.

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
- **Bounded tool-arg schemas.** A discovered tool's JSON-Schema `inputSchema` is
  mapped into the typed client-side arg gate only up to a bounded nesting depth, and
  a schema carrying an **external `$ref`** is refused (the arg still passes through
  for the server to validate) — preventing a malicious schema from driving an
  SSRF/fetch or a deep-recursion DoS.

### What's still coming

OAuth / device-flow setup and a hosted credential marketplace are a **Cloud**
capability (the OSS console shows them as an honest-disabled affordance). The
[`tool()` chains-node](#authoring-a-tool-step) is now live (call a discovered tool
from an authored DAG, across Python / TypeScript / CLI / the console builder). The
autonomous ReAct loop's **auto-grant** of dialed tools (the model auto-picking from
the live dialed set) lands in the next batch (it rebuilds the react warrant live
from the registry). **Parallel tool fan-out** (N concurrent tool calls per turn)
arrives with the embeddable agent-runner. (Branched write-back over the
content-addressed store is the D155 post-RC epic.)
