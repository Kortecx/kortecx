# @kortecx/ui — runtime console (T3.2 shell)

A static-hostable **React + Vite** single-page app over the frozen `KxGateway`
contract, talking **gRPC-web** to a running `kx serve` via the existing
[`@kortecx/sdk`](../bindings/typescript) browser client. No server tier: the
journal is the source of truth and the client is stateless (D119).

This is the **shell** (milestone T3.2 on the OSS UI critical path): connect to a
gateway, submit a run, and **watch its Motes transition live** in a status table
that the gateway is polled for. The next milestone (T3.3) swaps that table for an
interactive XYFlow DAG — reusing this exact data layer unchanged.

## Architecture

- **Routing/state:** TanStack Router (typed routes + search params) + TanStack
  Query (server-state, polling, caching, retry).
- **Live updates:** poll `GetProjection` over gRPC-web on an interval (TanStack
  Query `refetchInterval`); polling stops once every Mote is terminal. This is the
  verified, secure path (same CORS + bearer + TLS surface as every other RPC; no
  second port, no plaintext `ws://`). Lower-latency WS/delta streaming is a
  documented scale-path follow-on.
- **Data layer (`src/kx/`)** is the forward seam: the run-detail route consumes one
  hook, `useProjection(instanceId)`. T3.3 changes only the *view* it feeds.
- **Security:** the bearer token lives in **memory only** (never `localStorage` /
  the bundle). CORS is enforced server-side (deny-by-default). The shell renders
  enum labels + hex ids only — never arbitrary Mote content (no XSS surface).

## Prerequisites

The UI depends on the SDK's built `dist/` (via a `file:` dependency). **Build the
SDK first:**

```sh
npm --prefix ../bindings/typescript ci
npm --prefix ../bindings/typescript run build
```

(Run from this `ui/` directory the paths are `../bindings/typescript`; from the repo
root drop the `../`.)

## Develop

```sh
npm ci                 # install (after the SDK dist/ exists)
npm run dev            # Vite dev server on http://localhost:5173
```

Run a gateway that allows the dev origin (deny-by-default CORS — name it exactly):

```sh
# from the repo root
cargo build -p kx-cli
target/debug/kx serve \
  --journal /tmp/kx.db --content /tmp/kx-blobs \
  --listen 127.0.0.1:50151 --dev-allow-local \
  --cors-origin http://localhost:5173
```

Then open the dev server, connect to `http://127.0.0.1:50151`, submit a run (the
built-in `kx/recipes/echo` recipe is pre-filled), and watch the Mote flip to
`COMMITTED`.

## Test

```sh
npm run lint           # biome
npm run typecheck      # tsc --noEmit
KX_BIN="$PWD/../target/debug/kx" npm test   # vitest: unit + component + contract
npm run build          # vite build → dist/
npm run test:e2e       # Playwright (chromium) — builds + previews + drives a real gateway
```

- **Unit/component** tests mock the client and cover every Mote state/nd_class,
  error → UI mapping, the poll re-render seam, and the "token is never persisted"
  invariant.
- **Contract** test (`// @vitest-environment node`, gated on `KX_BIN`) drives a real
  `kx serve` and asserts byte-parity with the `kx` CLI + the auth/permission edges.
- **E2E** (Playwright) proves the real browser gRPC-web + CORS path end to end.

The agentic-shaper-children path needs on-device inference (Metal) and is exercised
**locally** only; CI uses the deterministic, no-model `kx/recipes/echo` path (SN-7).

## Scale note

The shell renders a plain (unvirtualized) table — comfortable to ~5k Motes, degrades
beyond. The runtime proves 25k-Mote *folding* (`scale-smoke`); windowing the table
(and then the DAG) with `@tanstack/react-virtual` lands in T3.3.
