# `kortecx` — Python client SDK

A **pure gRPC client SDK** for the [kortecx](https://github.com/Kortecx/kortecx)
durable agentic-execution runtime. It speaks the **frozen `KxGateway` contract**
(`kortecx.v1`) over the wire to a running `kx serve` — it is **not** a native
binding: no Rust, no C++, no `kx-*` crate, no compiler needed to install.

> **Note on the old design.** Earlier drafts of this directory described a
> PyO3 / `maturin` *native* binding. That approach is **superseded**: the
> supported integration surface is this gRPC client SDK (decision D130.3). The
> runtime is reached over its network contract, not linked in-process.

## Install

```bash
pip install kortecx            # core client (grpcio + protobuf)
pip install 'kortecx[ws]'      # + the optional WebSocket live-tail client
```

Run a gateway to talk to (FFI-free, no toolchain):

```bash
cargo install --path crates/kx-cli      # or use a prebuilt `kx` (install script, A0)
kx serve --journal /tmp/kx.db --content /tmp/kx-blobs --dev-allow-local
```

## Quickstart — call the runtime like a function

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151") as kx:
    # invoke a published recipe, wait for the durable result, get the bytes:
    result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
    print(result.text)             # the committed output
    print(result.instance_id)      # server-derived run id (hex)
    print(result.terminal_mote_id) # server-derived sink Mote (hex)
```

Async (asyncio):

```python
from kortecx import AsyncKxClient

async with AsyncKxClient("http://127.0.0.1:50151", token="…") as kx:
    result = await kx.invoke("kx/recipes/echo", {"topic": "hi"}, wait=True)
```

## Surface

A thin, typed mirror of the **frozen `KxGateway` contract** — the same surface the
`kx` CLI drives, with language-native ergonomics. Both `KxClient` (sync) and
`AsyncKxClient` (async) wrap **every one of the contract's 98 RPCs** (95 unary + 3
server-streaming), so the SDK is a complete client, not a subset. The headline
ergonomics:

- **`invoke(handle, args, wait=False, timeout=120)`** — bind a published recipe to
  JSON `args` and run it. With `wait=True` you get the committed `Result`
  (poll, or `wait_mode="events"` for a sub-second live subscription); without it,
  a `Run` handle. *The one-call "runtime as a function".*
- **`Run`** handle — `.wait()`, `.projection(at_seq=…)`, `.content(ref)`,
  `.events(since=0, follow=False)`, `.result()`.
- **Auth & TLS** — bearer token via `token=…`, `token_file=…`, or the `KX_TOKEN`
  env var (file/env preferred). An `https://` endpoint uses TLS automatically
  (a `grpc.secure_channel` with the platform trust store); a warning fires if a
  token would cross a non-loopback **plaintext** `http://` endpoint.
- **Events** — `stream_events(...)` yields typed deltas, auto-resumes from
  `next_seq`, and transparently reconnects on a `CatchupRequired` drop. An optional
  WebSocket client (`kortecx[ws]`) consumes the same live tail in browser/firewall
  -friendly JSON.
- **Typed errors** — every failure is a `KxError` subclass with a stable `.code`
  (`KxUnauthenticated`, `KxPermissionDenied`, `KxCatchupRequired`, `KxWaitTimeout`,
  `KxRunFailed`, …) mapped from the gateway's gRPC status.

### The full capability map

The one-call `invoke` / `chat` / `flow(...)` ergonomics sit on top of the *complete*
contract. Beyond durable submission + reads, the client exposes flat methods plus a
few verb-namespaces that mirror the `kx` CLI:

| Area | Surface |
|---|---|
| **Durable execution (core)** | `submit_run`, `invoke`, `submit_workflow`, `propose_workflow`, `run_chain`, `get_projection`, `get_content`, `get_content_batch`, `put_content`, `get_mote_detail`, `get_run_inputs`, `list_runs` |
| **Live events** | `stream_events`, `stream_all_events` (cross-run tail), `stream_model_tokens` (advisory token stream) |
| **Recipes & discovery** | `list_recipes`, `get_recipe_form`, `search_recipes`, `list_signatures` / `get_signature` / `register_signature` |
| **Apps** | `save_app`, `list_apps`, `get_app`, `get_app_manifest`, `run_app`, `scaffold_app` / `get_scaffold_status`, `lock_app` / `unlock_app`, `export_app_bundle` / `import_app` |
| **Chains & flows** | `chain(...)` string DSL + `flow()` builder → `run_chain` / `submit_workflow` (client-side lowering in `kortecx.chains` / `kortecx.flow`) |
| **Agentic patterns** | `flow().agent(...)`, `swarm(...)`, `team(...)`, `supervisor(...)`, `consensus(...)`, `map_reduce(...)`, `review_loop(...)`, `Agent`, `persona(...)` |
| **Agentic observability** | `list_react_turns`, `list_replan_rounds`, `list_rerank_turns`, `score_run` |
| **Models** | `list_models`, `load_model` / `offload_model`, `pull_model` / `get_pull_status`, `set_active_model` |
| **Memory** (`kx.memory`) | `store` / `list` / `recall` / `forget` / `decay` / `stats` / `restore` / `consolidate` |
| **HITL approvals** (`kx.approvals`) | `list_pending` / `grant` / `deny` |
| **Cost** (`kx.cost`) · **Eval** (`kx.eval`) | `get_run_cost` (per-run local spend estimate) · `score_run` (expectation-free quality readout) |
| **Tools + MCP** (`kx.connections`) | `register_tool` / `deregister_tool` / `discover_tools`, `list_tool_manifests` / `score_task_bundle`, connector `add` / `list` / `test` / `discover` / `remove` / `fire`, `call_mcp_tool` |
| **Datasets (RAG)** | `list_datasets`, `ingest_documents`, `query_dataset`, `fuzzy_discovery` |
| **Triggers** (`kx.triggers`) | `add` / `list` / `test` / `fire` / `remove` (webhook / cron / gRPC event ingress) |
| **Secrets** (`kx.secrets`) | `set` / `list` / `remove` (names + audit timestamps only; the value never returns, D81) |
| **Branches** | `create_branch`, `snapshot_into`, `list_branches`, `get_branch`, `delete_branch`, `advance_branch`, `get_branch_content` |
| **Context bundles** | `put_context_bundle`, `list_context_bundles`, `get_context_bundle`, `delete_context_bundle` |
| **Feedback** | `submit_feedback`, `list_feedback` |
| **Telemetry & alerts** | `list_mote_telemetry`, `list_telemetry_summary`, `list_alerts`, `list_capture_records` |
| **Teams & grants** | `list_teams`, `list_team_members`, `list_asset_grants` (read-only viewers) |
| **Skills** (`kx.skills`) | `add` / `list` / `show` / `remove` |
| **Server info** | `get_server_info` (non-secret resolved config + auth/TLS posture) |

`AsyncKxClient` exposes the same surface with `await`. Every id you pass is one the
runtime handed you back — identity is **server-derived** (SN-8); the SDK never mints one.

## Identity is server-derived (SN-8)

Every `MoteId` / `instance_id` / `content_ref` / `terminal_mote_id` is computed by
the runtime. The SDK **never** constructs one — it only carries the server's bytes
(surfaced as lowercase hex). This is a load-bearing security property; the client is
never a source of identity.

## Generated stubs

The protobuf + gRPC stubs under `src/kortecx/v1/` are generated from the single
source of truth — `crates/kx-proto/proto/` — by `./codegen.sh`, and are **committed**
(so `pip install` needs no `protoc`). CI's `codegen-fresh` guard re-runs codegen and
fails on any drift from the frozen proto.

## Develop

```bash
uv venv --python 3.12 .venv && source .venv/bin/activate
uv pip install -e '.[dev]'
./codegen.sh        # regenerate stubs (only if the proto changed)
pytest              # unit + contract e2e (spins up a real `kx serve`)
ruff check . && mypy src
```

Licensed under Apache-2.0.
