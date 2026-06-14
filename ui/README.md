# @kortecx/ui — runtime console (live DAG)

> **End users don't need any of this**: the prebuilt `kx` (the `curl|sh` install)
> ships this console **embedded** — `kx serve` hosts it at
> `http://127.0.0.1:50180` with zero node/npm (D139; `--console-listen` /
> `--no-console` to override). This README is the **developer** workflow (Vite
> dev server + HMR against a local gateway).

A static-hostable **React + Vite** single-page app over the frozen `KxGateway`
contract, talking **gRPC-web** to a running `kx serve` via the existing
[`@kortecx/sdk`](../bindings/typescript) browser client. No server tier: the
journal is the source of truth and the client is stateless (D119).

Connect to a gateway, run a blueprint, and **watch its Motes execute** — as a live
**XYFlow (reactflow) DAG** (nodes = Motes colored by state/`nd_class`, edges =
`parents[]`) or a status table, both polled live from `GetProjection`. This is the
T3.3 milestone on the OSS UI critical path (the live-DAG viewer over the T3.2 shell).

## Architecture

- **Routing/state:** TanStack Router (typed routes + search params) + TanStack
  Query (server-state, polling, caching, retry).
- **Live updates:** poll `GetProjection` over gRPC-web on an interval (TanStack
  Query `refetchInterval`). Polling stops authoritatively when the blueprint's
  **terminal (sink) Mote** commits — its id is threaded from the `Invoke` response
  through the route (`?terminal=…`) — so a multi-node run keeps updating while its
  Mote set is still growing (a naïve "all visible Motes terminal" check stops too
  early). A direct-URL navigation (no terminal id) falls back to frontier stability
  (all-terminal + `current_seq` unchanged). Same CORS + bearer + TLS surface as every
  other RPC; no second port, no plaintext `ws://`.
- **The DAG (`src/components/dag/`):** pure modules — `dag-graph` (projection →
  nodes/edges + a topology hash), `layout` (dagre), `edges` (DATA solid / CONTROL
  dashed / non-cascade dimmed), `flow` (reactflow adapters) — and thin React wrappers
  (`MoteNode`, `MoteDag`). Layout is memoized on the topology hash, so a state-only
  poll re-colors nodes in place without a relayout; new nodes (dynamic shaper
  children) animate in. reactflow + dagre are **code-split** into the run-detail route.
- **Data layer (`src/kx/`)** is the seam both views share: `useProjection`. The
  Graph/Table toggle, `?atSeq` time-travel, and Refresh are all view-agnostic. Above
  500 Motes the DAG falls back to the table (the scale surface).
- **Security:** the bearer token lives in **memory only** (never `localStorage` /
  the bundle). CORS is enforced server-side (deny-by-default). The console renders
  enum labels + server-derived hex ids only — never arbitrary Mote content (SN-8;
  no XSS surface).

## Prerequisites

The UI depends on the SDK's built `dist/` (via a `file:` dependency). **Build the
SDK first:**

```sh
npm --prefix ../bindings/typescript ci
npm --prefix ../bindings/typescript run build
```

(From this `ui/` directory the path is `../bindings/typescript`; from the repo root
drop the `../`.)

## Run it locally

**1. Start a gateway** that allows the dev origin (deny-by-default CORS — name it
exactly), from the repo root:

```sh
cargo build -p kx-cli
target/debug/kx serve \
  --journal /tmp/kx.db --content /tmp/kx-blobs \
  --listen 127.0.0.1:50151 --dev-allow-local \
  --cors-origin http://localhost:5173
```

**2. Start the UI** (from `ui/`):

```sh
npm ci                 # install (after the SDK dist/ exists)
npm run dev            # Vite dev server on http://localhost:5173
```

**3. In the browser** open <http://localhost:5173>, connect to
`http://127.0.0.1:50151`, then run a blueprint and watch the DAG execute:

- **`kx/recipes/echo`** (pre-filled, args `{"topic":"hello"}`) — a single COMMITTED node.
- **`kx/recipes/passthrough-dag`** (args `{}`) — a **5-node** fan-out → gather DAG
  (root → 3 children → gather) that runs model-free and lights up to COMMITTED. The
  best way to see the graph view.

Toggle **Graph / Table**, or pin a snapshot with `?atSeq=<n>` for time-travel.

## Test

```sh
npm run lint           # biome
npm run typecheck      # tsc --noEmit
KX_BIN="$PWD/../target/debug/kx" npm test   # vitest: unit + component + contract
npm run build          # vite build → dist/ (reactflow/dagre code-split)
npm run test:e2e       # Playwright (chromium) — builds + previews + drives a real gateway
```

- **Unit/component** tests cover the pure DAG modules across every topology
  (chain / diamond / fan-out / fan-in / disconnected / control-edge / cycle-guard),
  the no-relayout-on-state-only-poll invariant, the dynamic-child-appearance path,
  the >500-node table fallback, error → UI mapping, and the poll-stop logic.
- **Contract** test (`// @vitest-environment node`, gated on `KX_BIN`) drives a real
  `kx serve`: echo's empty `parents[]` proves the edge wire end-to-end, `passthrough-dag`
  proves a real multi-node `parents[]` DAG, and byte-parity with the `kx` CLI holds.
- **E2E** (Playwright) proves the real browser gRPC-web + CORS path: echo reaching
  COMMITTED in the DAG, and the `passthrough-dag` graph rendering all 5 nodes COMMITTED.

The agentic-shaper-children path needs on-device inference (Metal) and is exercised
**locally** only; CI uses the deterministic, no-model `echo` + `passthrough-dag` paths (SN-7).

## Scale note

The DAG renders for runs up to **500 Motes** (above that it falls back to the table,
which is comfortable to ~5k and degrades beyond). The runtime proves 25k-Mote
*folding* (`scale-smoke`); windowing the table/DAG with `@tanstack/react-virtual` is
a documented follow-on.
