---
id: authoring-a-connector
title: Authoring a connector
sidebar_label: Authoring a connector
description: Write an external MCP connector, register it through the single chaining entry point (flow().with_mcp / kx connections), and prove it safe with the kx-extension-sdk conformance gate.
---

# Authoring a connector

A **connector** is an external [MCP](https://modelcontextprotocol.io) tool server — a
separate process the runtime dials over **stdio** (a subprocess) or **Streamable-HTTP**.
It is how you extend a `kx serve` runtime with new tools: the runtime stays a *secure
gateway* (it never runs your code in-process), discovers your tools, and lets the
agentic loop fire them **only under a warrant that grants them** (SN-8 — the model
proposes, the runtime enforces).

The authoring surface is curated + semver-pinned in the
[`kx-extension-sdk`](https://docs.rs/kx-extension-sdk) crate (D167 E0).

## The chaining entry point

Connectors are reachable from the **same single chaining entry point** as every other
capability — register one inline with `with_mcp` and reference its namespaced
`<server>/<tool>` tools in the next step:

```python title="Python"
import kortecx as kx

out = (kx.flow()
       .with_mcp("fs", endpoint="npx",
                 args=["-y", "@modelcontextprotocol/server-filesystem", "/data"])
       .agent("Summarise the files in /data", tools=["fs/list_directory", "fs/read_text_file"])
       .run())
print(out.text)
```

```ts title="TypeScript"
import { flow } from "@kortecx/sdk";

const out = await flow()
  .withMcp({ name: "fs", endpoint: "npx",
             args: ["-y", "@modelcontextprotocol/server-filesystem", "/data"] })
  .agent("Summarise the files in /data", { tools: ["fs/list_directory", "fs/read_text_file"] })
  .run({ client: kx });
console.log(out.text);
```

`with_mcp` / `withMcp` is pure pre-submit sugar over `register_mcp_server`: it registers
the connector **before** the flow submits (so the referenced tools resolve), then submits
the unchanged workflow. It adds no node to the lowered graph — the canonical digest is
invariant — and is idempotent (re-running is safe).

## Registering without a flow

The same registration is available imperatively across every surface — pick whichever
fits (the flow sugar above just calls the SDK method for you):

```bash title="CLI (operator)"
kx connections add --name fs --command "npx -y @modelcontextprotocol/server-filesystem /data"
kx connections list
kx agent run --goal "list /data" --tools fs/list_directory
```

```python title="Python — kx.connections namespace"
kx.connections.add("fs", endpoint="npx",
                   args=["-y", "@modelcontextprotocol/server-filesystem", "/data"])
kx.connections.list()
kx.connections.discover("fs")      # re-dial + list the tools it exposes
kx.connections.test("fs")          # reachability probe
kx.connections.remove("fs")
```

```ts title="TypeScript — kx.connections namespace"
await kx.connections.add({ name: "fs", endpoint: "npx",
  args: ["-y", "@modelcontextprotocol/server-filesystem", "/data"] });
await kx.connections.list();
await kx.connections.discover("fs");
```

Every discovered tool is namespaced `<server>/<remote>` (server-derived, SN-8), so tools
from different connectors never collide.

## What a connector must implement

A connector speaks newline-delimited JSON-RPC 2.0 over its transport and implements the
full MCP lifecycle so the runtime can **discover** its tools at registration:

| Method       | The runtime expects |
|--------------|---------------------|
| `initialize` | a `protocolVersion` + `serverInfo` |
| `tools/list` | the tool manifests (`name`, `description`, JSON-Schema `inputSchema`) |
| `tools/call` | the tool result (or a JSON-RPC error — fail closed) |

The SDK ships a minimal, complete **reference connector** to copy from —
`kx-connector-example` (`crates/kx-extension-sdk/src/bin/reference_connector.rs`): two
pure tools (`echo`, `reverse`), full handshake, no environment echo.

:::tip Security contract
- **Never echo your environment.** An injected credential (see below) must reach no
  reply, so it never lands in a journal/content/telemetry sink (D81).
- **Fail closed.** An unknown method / bad args ⇒ a JSON-RPC error, surfaced as a
  capability failure — never a fabricated success.
:::

## Secrets by reference (D81)

A credential is referenced by **name**, never by value. The name (an env var / vault
key) is resolved transiently at dial and injected into the transport; it reaches no
durable sink:

```python
# the secret VALUE lives only in the runtime's environment; only its NAME travels
kx.connections.add("gh", endpoint="npx", args=["-y", "@some/github-mcp"],
                   credential_ref="GITHUB_TOKEN")
```

## The warrant gate (SN-8)

A registered tool fires **only** through a warrant that grants its `(name, version)` and
whose scopes cover the tool's `required_capability`. An HTTP connector's tools are egress
to **only** their server's host (two-gate SSRF vetting at admission + dial). Mere presence
in the registry never fires anything.

## Proving a connector is safe

Run the conformance gate — it dials your connector through the **real** gateway path and
mechanizes a subset of the Extension Acceptance Gate:

```bash
just test-connector ./my-mcp-server --some-flag        # a stdio server
just test-connector https://mcp.example.com/rpc        # a Streamable-HTTP server
just test-connector                                    # the bundled reference connector
```

| Gate item | What it asserts |
|-----------|-----------------|
| 3 — out-of-process | every discovered tool registers as `ToolKind::Mcp` (external), never `Builtin` |
| 5 — warrant / SN-8 | a no-grant warrant + a wrong-tool grant are both refused; a correct grant passes the gate |
| 7 — secret-by-ref  | an out-of-band credential reaches no sink (payload / handle / staged result / MoteId) |
| 10 — on / off      | the tool is fail-closed when the connector is absent; registering adds exactly its tools |

Rust authors can call the harness directly from their own tests:

```rust
use kx_extension_sdk::conformance::{run_conformance, ConnectorUnderTest};
use kx_extension_sdk::prelude::{SessionMode, TransportSpec};

let report = run_conformance(&ConnectorUnderTest {
    name: "my-server".into(),
    transport: TransportSpec::Stdio { command: "./my-mcp-server".into(), args: vec![] },
    credential_ref: None,
    session_mode: SessionMode::Stateless,
});
assert!(report.passed(), "{report:#?}");
```

The `kx_extension_sdk::prelude` re-exports the load-bearing seams (the dial path, the tool
vocabulary, the warrant boundary, the transports, the secret types) as one curated,
semver-pinned import for Rust connector authors.
```rust
use kx_extension_sdk::prelude::*;
```

## OSS / Cloud line

OSS dials local + first-party connectors (one App at a time). Multi-tenant connector
registries, a connector marketplace, and a hardened secrets vault are Cloud concerns
(D129).
