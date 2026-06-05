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

A thin, typed mirror of the `KxGateway` contract — exactly what the `kx` CLI does,
with language-native ergonomics. Both `KxClient` (sync) and `AsyncKxClient` (async)
expose:

- **`invoke(handle, args, wait=False, timeout=120)`** — bind a published recipe to
  JSON `args` and run it. With `wait=True` you get the committed `Result`
  (poll, or `wait_mode="events"` for a sub-second live subscription); without it,
  a `Run` handle. *The one-call "runtime as a function".*
- **`Run`** handle — `.wait()`, `.projection(at_seq=…)`, `.content(ref)`,
  `.events(since=0, follow=False)`, `.result()`.
- **All eight RPCs** — `submit_run`, `invoke`, `get_projection`, `get_content`,
  `stream_events`, `list_signatures`, `get_signature`, `register_signature`.
- **Auth** — bearer token via `token=…`, `token_file=…`, or the `KX_TOKEN` env var
  (file/env preferred; a warning fires if a token would cross a non-loopback
  plaintext endpoint — TLS lands in a later release).
- **Events** — `stream_events(...)` yields typed deltas, auto-resumes from
  `next_seq`, and transparently reconnects on a `CatchupRequired` drop. An optional
  WebSocket client (`kortecx[ws]`) consumes the same live tail in browser/firewall
  -friendly JSON.
- **Typed errors** — every failure is a `KxError` subclass with a stable `.code`
  (`KxUnauthenticated`, `KxPermissionDenied`, `KxCatchupRequired`, `KxWaitTimeout`,
  `KxRunFailed`, …) mapped from the gateway's gRPC status.

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
