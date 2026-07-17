# `@kortecx/sdk` — TypeScript / JavaScript client SDK

A **pure gRPC + gRPC-web client** over the frozen `KxGateway` contract — **NOT a
native FFI binding**. It links no Rust, no C++, no `kx-*` crate; it speaks
protobuf over the wire to a running `kx serve`. The generated stubs are vendored
under `src/gen/` (regenerate with `./codegen.sh`), so `npm install` needs no
`protoc`.

One typed `KxClient` runs in **both Node.js and the browser** (it is the data
layer for the kortecx dashboard) — only the transport differs:

| Runtime | Import | Transport |
|---|---|---|
| Node.js (tooling, servers, agents) | `@kortecx/sdk` or `@kortecx/sdk/node` | gRPC over HTTP/2 (`@connectrpc/connect-node`) — talks to today's gateway unchanged |
| Browser (dashboards) | `@kortecx/sdk/web` | gRPC-web (`@connectrpc/connect-web`) + the R5 WebSocket live-tail |

## Install

```bash
npm install @kortecx/sdk
# the optional WebSocket live-tail in Node needs `ws`:
npm install ws
```

## Quickstart

Start a gateway, then call it like a function:

```bash
kx serve --dev-allow-local --journal /tmp/kx.db --content /tmp/kx-blobs
```

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151");
const result = await kx.invoke("kx/recipes/echo", { topic: "hello" }, { wait: true });
console.log(result.state, result.instanceId, result.text);
kx.close();
```

Stream events with a native async iterator:

```ts
const run = await kx.invoke("kx/recipes/echo", { topic: "watch me" }); // a Run handle
for await (const delta of run.events({ since: 0n, follow: false })) {
  console.log(delta.seq, delta.kind, delta.moteId);
}
```

See [`examples/`](./examples) for `invoke_wait.ts`, `stream_events.ts`,
`async_invoke.ts` (low-latency `waitMode: "events"`), and `browser_events.html`.

## API surface

One async-first `KxClient` wraps **every one of the frozen `KxGateway` contract's 98
RPCs** (95 unary + 3 server-streaming) — a complete client, not a subset — with a
high-level `invoke(..., { wait: true })` on top:

- `invoke(handle, args, opts?)` → a `Run` handle, or (with `wait: true`) a `Result`
- `submitRun(request, opts?)` — low-level propose-proxy submit
- `getProjection(instanceId, { atSeq? })` → `Projection` (the run as a DAG)
- `getContent(ref, instanceId)` → `Uint8Array`
- `streamEvents(instanceId, { since?, follow?, signal? })` → `AsyncIterable<Delta>`
- `wsEvents(instanceId, { since?, wsEndpoint? })` → `AsyncIterable<Delta>` (R5 WS bridge)
- `listSignatures()` / `getSignature(id)` / `registerSignature(manifest)`

A `Run` handle exposes `.wait()`, `.result()`, `.projection()`, `.content()`,
`.events()`. A `Result` exposes `.ok`, `.text`, `.bytes`, `.toJSON()` (the same
shape as `kx … --wait --json`).

### The full capability map

The `invoke` / `chat` / `flow(...)` ergonomics sit on the *complete* contract. Beyond
durable submission + reads, `KxClient` exposes flat camelCase methods plus verb
namespaces that mirror the `kx` CLI (`kx.memory`, `kx.secrets`, `kx.triggers`,
`kx.approvals`, `kx.cost`, `kx.eval`, `kx.connections`, `kx.skills`):

| Area | Surface |
|---|---|
| **Durable execution (core)** | `submitRun`, `invoke`, `submitWorkflow`, `proposeWorkflow`, `runChain`, `getProjection`, `getContent`, `getContentBatch`, `putContent`, `getMoteDetail`, `getRunInputs`, `listRuns` |
| **Live events** | `streamEvents`, `streamAllEvents` (cross-run tail), `streamModelTokens` (advisory token stream), `wsEvents` (browser WS bridge) |
| **Recipes & discovery** | `listRecipes`, `getRecipeForm`, `searchRecipes`, `listSignatures` / `getSignature` / `registerSignature` |
| **Apps** | `saveApp`, `listApps`, `getApp`, `getAppManifest`, `runApp`, `scaffoldApp` / `getScaffoldStatus`, `lockApp` / `unlockApp`, `exportAppBundle` / `importApp` / `cloneApp` |
| **Chains & flows** | `chain(...)` string DSL + `chainFrom(...)` combinators + `flow()` builder → `runChain` / `submitWorkflow` (`@kortecx/sdk/chains`) |
| **Agentic patterns** | `flow().agent(...)`, `swarm(...)`, `team(...)`, `supervisor(...)`, `consensus(...)`, `mapReduce(...)`, `fanOutGather(...)`, `reviewLoop(...)`, `Agent`, `persona(...)` |
| **Agentic observability** | `listReactTurns`, `listReplanRounds`, `listRerankTurns`, `scoreRun` |
| **Models** | `listModels`, `loadModel` / `offloadModel`, `pullModel` / `getPullStatus`, `setActiveModel` |
| **Memory** (`kx.memory`) | `store` / `list` / `recall` / `forget` / `decay` / `stats` / `restore` / `consolidate` |
| **HITL approvals** (`kx.approvals`) | `listPending` / `grant` / `deny` |
| **Cost** (`kx.cost`) · **Eval** (`kx.eval`) | `getRunCost` (per-run local spend estimate) · `scoreRun` (expectation-free quality readout) |
| **Tools + MCP** (`kx.connections`) | `registerTool` / `deregisterTool` / `discoverTools`, `listToolManifests` / `scoreTaskBundle`, connector `add` / `list` / `test` / `discover` / `remove` / `fire`, `callMcpTool` |
| **Datasets (RAG)** | `listDatasets`, `ingestDocuments`, `queryDataset`, `fuzzyDiscovery` |
| **Triggers** (`kx.triggers`) | `add` / `list` / `test` / `fire` / `remove` (webhook / cron / gRPC event ingress) |
| **Secrets** (`kx.secrets`) | `set` / `list` / `remove` (names + audit timestamps only; the value never returns) |
| **Branches** | `createBranch`, `snapshotInto`, `listBranches`, `getBranch`, `deleteBranch`, `advanceBranch`, `getBranchContent` |
| **Context bundles** | `putContextBundle`, `listContextBundles`, `getContextBundle`, `deleteContextBundle` |
| **Feedback** | `submitFeedback`, `listFeedback` |
| **Telemetry & alerts** | `listMoteTelemetry`, `listTelemetrySummary`, `listAlerts`, `listCaptureRecords` |
| **Teams & grants** | `listTeams`, `listTeamMembers`, `listAssetGrants` (read-only viewers) |
| **Skills** (`kx.skills`) | `add` / `list` / `show` / `remove` |
| **Server info** | `getServerInfo` (non-secret resolved config + auth/TLS posture) |

Every id you pass is one the runtime handed you back — identity is **server-derived**
(SN-8); the SDK never mints one.

### Parity with the Python SDK

The surface mirrors [`bindings/python`](../python) one-to-one, with idiomatic TS
mappings:

| Python | TypeScript |
|---|---|
| `KxClient` (sync) + `AsyncKxClient` | one async-first `KxClient` (Promises) |
| `bytes` | `Uint8Array` |
| `uint64` seq fields (int) | `bigint` (`0n`, `result.committedSeq`) |
| `result.to_dict()` | `result.toJSON()` (byte-comparable to the CLI) |
| `ErrorCode` (str enum) | `ErrorCode` (string enum, **identical values**) |
| `for d in run.events(...)` | `for await (const d of run.events(...))` |

### Errors

Every failure is a typed `KxError` with a stable `ErrorCode` whose string values
are **identical across the Python SDK, this SDK, and the CLI `--json`** surface —
branch on `err.code`, not on messages:

```ts
import { KxPermissionDenied, ErrorCode } from "@kortecx/sdk";

try {
  await kx.getProjection(someInstanceId);
} catch (err) {
  if (err instanceof KxPermissionDenied) { /* uniform: no existence oracle */ }
  // or: if (err.code === ErrorCode.PermissionDenied) { … }
}
```

`KxCatchupRequired` carries `.nextSeq`; `KxWaitTimeout` / `KxRunFailed` carry
`.instanceId` / `.terminalMoteId` (the run stays resumable).

## Auth & TLS

Pass a bearer token; the caller's party is **server-derived** from it (SN-8 — the
client never asserts an identity or computes an id):

```ts
const kx = new KxClient("https://gateway.example.com:50151", { token: "…" });
// Node only: { tokenFile: "/run/secrets/kx-token" } or the KX_TOKEN env var.
```

`https://` uses TLS automatically. A bearer token sent over plaintext `http://` to
a non-loopback host emits a `console.warn`. In the browser, the WS bridge carries
the token in the `Sec-WebSocket-Protocol: bearer, <token>` subprotocol. **Browser
tokens are visible to page JS — use them only for trusted first-party dashboards
with short-lived, scoped tokens over `https://` / `wss://`.**

## Browser notes

`wsEvents` (the R5 WS bridge) works against today's gateway and is the browser
live-tail path. Unary gRPC-web from a browser additionally needs a server-side
grpc-web / CORS handler on the gateway (a follow-up gateway PR); until it lands,
drive reads from Node, or use `wsEvents` for live updates.

## Develop

```bash
npm ci
./codegen.sh        # regenerate src/gen from ../../crates/kx-proto/proto
npm run typecheck   # tsc --noEmit
npm run lint        # biome ci
npm test            # vitest (unit + contract e2e against a real `kx serve`)
npm run build       # tsup → dual ESM + CJS + .d.ts
```

The contract tests spawn a real `kx serve` (built FFI-free) and assert the SDK
reproduces `kx invoke --wait --json` **byte-for-byte**. CI's `codegen-fresh` guard
re-runs `./codegen.sh` and fails on any drift from the frozen proto.

## License

Apache-2.0.
