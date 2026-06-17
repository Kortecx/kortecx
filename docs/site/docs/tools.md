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

`RegisterTool` records a tool and its **SSRF-vetted** egress host. The host is
checked at admission (deny-by-default — internal / loopback / link-local /
metadata endpoints are refused); an optional operator allowlist is
`KX_SERVE_TOOL_HOST_ALLOWLIST`. Registration **does not dial** the host — dialing
external MCP servers is a [PR-6b](#whats-next-pr-6b--the-external-mcp-gateway)
capability.

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

## OSS / Cloud line

Kortecx OSS is the **secure gateway to external MCP** — the runtime is the
production-grade authority that connects to external MCP servers (which host the
tools). **No arbitrary code runs in the runtime.** In-runtime sandboxed
script/skill execution, and OAuth / hosted-marketplace credential connections, are
Cloud / future capabilities.

## What's next (PR-6b) — the external MCP gateway

The registry stores a vetted `server_host` today; **dialing** it lands in PR-6b:
a multi-server MCP gateway (stdio + Streamable HTTP) with per-server health,
discovery into the same registry, a dial-time SSRF gate, per-server
rate-limit/quota, warrant-gated egress, and secret-less `CredentialRef`
**Connections** — plus the `tool()` chains-node so an authored DAG can call a tool
directly. The Connections card in the console is the honest forward stub for that
work. (Branched write-back over the content-addressed store is the D155
post-RC epic.)
