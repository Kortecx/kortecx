# kortecx

> The durable runtime for AI agents.
> **Knowledge → Intelligence.**

🌐 **[kortecx.com](https://kortecx.com)** &nbsp;·&nbsp; built in the open at [Kortecx/kortecx](https://github.com/Kortecx/kortecx)

[![CI](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml)
[![Private-content leak check](https://github.com/Kortecx/kortecx/actions/workflows/leak-check.yml/badge.svg?branch=main)](https://github.com/Kortecx/kortecx/actions/workflows/leak-check.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![MSRV](https://img.shields.io/badge/MSRV-1.94.0-orange.svg)](rust-toolchain.toml)
[![Rust Edition](https://img.shields.io/badge/Rust-2021-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/index.html)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue.svg)](#)
[![Status](https://img.shields.io/badge/status-early%20development-yellow.svg)](#)

kortecx runs AI agents you can trust with real work. One small binary gives you
**durable, exactly-once agentic execution** — live agent loops that plan,
re-plan, self-check, and call tools; reusable **Blueprints**; **RAG datasets**;
**local LLM inference**; a built-in **web console**; and **Python/TypeScript
SDKs** — all over an append-only journal that survives crashes and never runs a
world-touching step twice.

```bash
curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
kx serve --journal /tmp/kx.db --content /tmp/kx-content --dev-allow-local
# → gRPC 127.0.0.1:50151 · events ws://127.0.0.1:50152 · web console http://127.0.0.1:50180
```

---

- [What you get](#what-you-get)
- [Install](#install) · [Prerequisites](#prerequisites)
- [Quick start: prove exactly-once](#quick-start-prove-exactly-once)
- [Start the runtime locally](#start-the-runtime-locally) — serve, run blueprints, chat, ReAct with tools
- [The web console](#the-web-console)
- [CLI reference](#cli-reference) — every command, flag, and environment variable
- [Blueprints](#blueprints) · [Datasets & RAG](#datasets--rag) · [SDKs](#sdks)
- [Security defaults](#security-defaults) · [Run in Docker](#run-in-docker)
- [Production notes](#production-notes)
- [Contributing](#contributing) · [License](#license) · [Links](#links)

---

## What you get

| Capability | What it does | Where |
|---|---|---|
| **Exactly-once agentic runs** | Every step (a *Mote*) commits durably to an append-only journal; crashes replay from committed work — a step that touched the world is re-read, never re-run | CLI · SDKs · console |
| **The live agent loop** | Models **plan** topology, **re-plan** on failure, pass **critic** gates, and run **ReAct turns with real MCP tools** — all inside `kx serve`, all crash-safe | `kx/recipes/react` + the console Chat |
| **Blueprints** | Reusable, parameterized workflows published by handle — pick one, fill its typed inputs, run it, watch the live DAG | CLI `kx invoke` · SDKs · console |
| **Local LLM inference** | Bring any fit GGUF model; on-device llama.cpp (Metal/CPU) drives chat + the agent loop — no API keys, no egress | `kx serve` (inference build) |
| **Datasets & RAG** | Ingest documents, search by vector similarity, ground agent runs — durable, content-addressed corpora | CLI/SDK/console Datasets |
| **Live events & time-travel** | Stream every state change as it commits; scrub any run back to any point in its history | `kx events --follow` · console Activity |
| **Run capture (Morphic)** | Every serve-path run's actions are captured to a durable sidecar — your agents' exhaust becomes queryable data | SDKs (`ListCaptureRecords`) |
| **Teams & grants** | Durable membership + asset grants with resolved-warrant views | console Systems · SDKs |
| **Audit trail** | An off-the-truth-path JSONL record of the run lifecycle | `kx run --audit-log` |
| **The web console** | All of the above in a browser — served by `kx` itself, zero extra setup | `http://127.0.0.1:50180` |

Every capability is reachable from the **CLI**, the **Python and TypeScript
SDKs**, and the **web console** — same wire, same guarantees.

## Install

```bash
# Prebuilt binary (Linux x86_64/arm64, macOS arm64) — SHA-256 verified, no sudo,
# installs to ~/.local/bin. The prebuilt ships the web console + Datasets built in.
curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
```

| Installer env | Meaning |
|---|---|
| `KX_VERSION` | install a specific release tag (default: latest) |
| `KX_INSTALL_DIR` | install directory (default `~/.local/bin`) |

From source (Rust 1.94+; each variant adds a capability):

```bash
git clone https://github.com/Kortecx/kortecx.git && cd kortecx
cargo install --path crates/kx-cli                          # the core runtime — no C++, no node
cargo install --path crates/kx-cli --features hnsw          # + Datasets/RAG (still no C++)
cargo install --path crates/kx-cli --features inference,hnsw  # + local LLM inference (needs a C++ toolchain)
just console-build                                          # + the embedded web console (needs node 22; repo checkout only)
```

> The web console is embedded at **compile time**, so `--features console` needs
> the built SPA (`just console-dist`) — use the prebuilt binary if you don't want
> node. Plain `cargo install` never needs node or C++.

## Prerequisites

| Tier | You get | You need |
|---|---|---|
| **Tier 0 — the runtime** | everything except on-device inference | nothing (prebuilt) or **Rust 1.94+** (source) |
| **Tier 1 — local LLM inference** | on-device model inference via llama.cpp | a **C++ toolchain** (CMake, clang/libclang) + a **GGUF** model |

Run **`just doctor`** (repo checkout) for a tiered preflight that prints the
exact install command for anything missing. Tier 1 hints — macOS:
`xcode-select --install && brew install cmake`; Debian/Ubuntu:
`sudo apt-get install -y cmake clang libclang-dev build-essential`.

## Quick start: prove exactly-once

Run the canonical demo workflow, crash it mid-commit, and replay. The digest is
identical across the clean run and the crash-then-replay run:

```bash
# 1. Run the demo to completion, capturing its deterministic digest.
kx run    --journal /tmp/kx.db --content /tmp/kx-content
#    → 7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9 (8/8 committed)

# 2. Start fresh, but hard-abort right after a side effect commits.
rm -f /tmp/kx.db; rm -rf /tmp/kx-content
kx run    --journal /tmp/kx.db --content /tmp/kx-content --crash-at post-commit-vtc

# 3. Recover from the journal and finish the run.
kx replay --journal /tmp/kx.db --content /tmp/kx-content
#    → same digest — the crashed step was re-read, not re-run.
```

Same digest = the exactly-once property, demonstrated. (`just verify-quickstart`
runs exactly this and asserts the digest, so these docs can't silently drift.)

## Start the runtime locally

**1. Serve.** One command starts the gateway, the embedded worker, the live-event
bridge, and (prebuilt binaries) the web console:

```bash
kx serve --journal /tmp/kx.db --content /tmp/kx-content --dev-allow-local
#    gRPC on 127.0.0.1:50151 · events on ws://127.0.0.1:50152
#    web console at http://127.0.0.1:50180  ← open this in your browser
```

**2. Run your first blueprints** (another terminal):

```bash
# A single-step echo — the canonical hello-world (typed input: topic).
kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait

# A real multi-node DAG, model-free: root → 3 children → gather (5 steps, all committed).
kx invoke kx/recipes/fanout-demo --args '{}' --wait
```

**3. Inspect anything** — the DAG, a committed result, the live event stream:

```bash
kx projection --instance <instance-id>                       # the run as a DAG of step states
kx projection --instance <instance-id> --at-seq 3            # …time-traveled to any point
kx content    --ref <content-ref> --instance <instance-id>   # a committed result (raw bytes)
kx events     --instance <instance-id> --follow              # live-tail the run's events
```

**4. Add a local model** (inference build — `--features inference,hnsw`). Download
any **fit** GGUF from Hugging Face and point the server at it. The runtime
validates fitness at startup: a chat template (ChatML), **native tool-calling**,
a commercial-friendly license, `q4_k_m`/`q8_0`/`f16` quantization, and a context
window ≥ 2048. The Qwen3 family fits; the 0.6B stand-in below runs on ~0.5 GB:

```bash
curl -fsSL -o qwen3-0.6b-q4_k_m.gguf \
  https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf
shasum -a 256 qwen3-0.6b-q4_k_m.gguf
#    → ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a

KX_SERVE_MODEL_GGUF="$PWD/qwen3-0.6b-q4_k_m.gguf" \
  kx serve --journal /tmp/kx.db --content /tmp/kx-content --dev-allow-local
```

**5. Chat and run the live agent loop** — real on-device inference, durable every
turn:

```bash
# One-shot chat: greedy decode over your model, committed like any other step.
kx invoke kx/recipes/chat --args '{"prompt":"What is the capital of France?"}' --wait

# A full ReAct agent: reason → call a tool → observe → answer, every turn a
# durable fact. The bundled mcp-echo@1 tool is wired in; budgets are yours:
kx invoke kx/recipes/react \
  --args '{"instruction":"Echo the word kortecx via your tool, then summarize.","max_turns":4,"max_tool_calls":2}' \
  --wait
```

Crash the server mid-run and start it again: the loop resumes from its committed
turns — that's the whole point.

## The web console

`kx serve` (prebuilt binaries) hosts the full console at
**`http://127.0.0.1:50180`** — no node, no separate install. Connect to your
gateway endpoint (pre-filled: `http://127.0.0.1:50151`; the bearer token, if you
use one, stays in browser memory and is never stored).

| Section | What you do there |
|---|---|
| **Chat** *(default)* | converse with the runtime — every turn a durable run |
| **Activity** | the live event feed, run metrics, and **time-travel** (scrub any run's history) |
| **Runs** | session + durable run history; open any run as a **live DAG** |
| **Blueprints** | the catalog — pick a blueprint, fill its typed form, run it |
| **Artifacts** | browse + review committed outputs |
| **Datasets** | ingest documents, search by similarity (RAG) |
| **Systems** | gateway health, teams, members, asset grants + resolved warrants |
| **Settings** | connection profile + console preferences |

Plus: **⌘K** jumps anywhere; the **DevTools dock** (navbar toggle) tails live
events and gateway health from any screen. Override the console address with
`--console-listen <addr:port>` (loopback only) or turn it off with
`--no-console`. For a remote browser, static-host `ui/dist` and grant its origin
with `--cors-origin`.

## CLI reference

`kx <command> --help` prints per-command usage. **Exit codes:** `0` success ·
`2` usage/config error · `3` `--wait` timed out (the run is still in progress
and resumable) · `1` everything else.

### Local engine (no server)

| Command | What it does |
|---|---|
| `kx run` | drive the canonical demo workflow from scratch |
| `kx replay` | recover an existing journal and finish the run |
| `kx digest` | print the projection digest of a journal |

All three take: `--journal <path>` (the SQLite journal) · `--content <dir>` (the
content store) · `--crash-at <point>` (deterministic crash injection, e.g.
`post-commit-vtc`) · `--checkpoint-every <N>` · `--audit-log <path>` (JSONL audit
trail) · `--json`.

```bash
kx run --journal /tmp/kx.db --content /tmp/kx-content --audit-log /tmp/kx-audit.jsonl
```

### `kx serve` — the runtime as a server

```bash
kx serve --journal <path> --content <dir> [flags]
```

| Flag | Default | Meaning |
|---|---|---|
| `--listen <addr:port>` | `127.0.0.1:50151` | the gRPC + gRPC-web endpoint |
| `--ws-listen <addr:port>` | `127.0.0.1:50152` | the live-event WebSocket bridge |
| `--console-listen <addr:port>` | `127.0.0.1:50180` | the embedded web console (loopback only) |
| `--no-console` | — | disable the web console |
| `--dev-allow-local` | off | dev auth: allow loopback callers (loopback binds only) |
| `--auth-token <token>=<party>` | — | accept a bearer token as a party (repeatable) |
| `--auth-token-file <path>` | — | `token=party` per line (`#` comments) |
| `--cors-origin <origin>` | deny all | allow a browser origin on the gRPC-web shim (repeatable, never a wildcard) |
| `--tls-cert <pem> --tls-key <pem>` | plaintext | in-binary TLS for the gRPC listener |
| `--catalog-dir <dir>` | beside the journal | durable catalog (blueprints, signatures, teams) |
| `--max-lease <N>` | `16` | embedded-worker lease batch size |

**Auth is deny-all by default** — pass `--dev-allow-local` (local development) or
bearer tokens. With no flags a `kx serve` answers nobody.

### Client commands

Every client command takes the **shared flags**: `--endpoint <url>` (default
`http://127.0.0.1:50151`) · `--token <t>` / `--token-file <p>` (bearer auth;
prefer the file — `--token` is visible in `ps`) · `--tls-ca <pem>` (for
`https://` endpoints) · `--json` (machine-readable output).

| Command | What it does | Flags |
|---|---|---|
| `kx invoke <handle>` | run a published blueprint to a committed result | `--args <json>` / `--args-file <path>` (exactly one) · `--wait` · `--timeout-secs <N>` (default 120) · `--out <file>` |
| `kx submit --demo` | submit the built-in pure demo run | `--wait` · `--timeout-secs` · `--out` |
| `kx projection` | render a run as a DAG of step states | `--instance <hex>` · `--at-seq <N>` (time-travel) |
| `kx content` | fetch a committed result (raw bytes, binary-safe) | `--ref <hex>` · `--instance <hex>` · `--out <file>` |
| `kx events` | print or live-tail a run's event deltas | `--instance <hex>` · `--since <N>` · `--follow` |
| `kx signatures` | the sharable task-signature catalog | `list` · `get --id <hex>` · `register --manifest-file <path>` |
| `kx tools` | advisory MCP-tool discovery + TaskBundle preview (scores never authorize) | `list` · `score --intent <text> --tool <id>@<ver>… [--language-tag <t>]… [--tolerance-threshold-bp <N>]` |
| `kx health` | gateway liveness (the standard gRPC health probe) | shared flags |
| `kx help [command]` / `kx --version` | usage / version | — |

```bash
# Run a blueprint and save the committed result bytes:
kx invoke kx/recipes/echo --args '{"topic":"hello"}' --wait --out /tmp/result.bin

# Everything speaks JSON for scripting:
kx invoke kx/recipes/echo --args '{"topic":"hello"}' --wait --json | jq .

# Discover the registered tools and preview a task bundle (advisory — never runs anything):
kx tools list --json | jq '.manifests[].tool_id'
kx tools score --intent "read a file from disk" --tool fs-read@1 --json | jq '.ranked, .verdict'
```

### Environment variables

| Variable | Used by | Meaning |
|---|---|---|
| `KX_SERVE_MODEL_GGUF` | `kx serve` (inference build) | absolute path to the GGUF model; enables `kx/recipes/chat` + `kx/recipes/react` |
| `KX_N_GPU_LAYERS` | inference | GPU offload layers (Metal/CUDA; default: all that fit) |
| `KX_FLASH_ATTN` | inference | enable flash attention |
| `KX_KV_TYPE` | inference | KV-cache quantization type |
| `KX_N_THREADS` | inference | CPU threads for inference |
| `KX_MCP_ECHO_PATH` | `kx serve` | override the bundled `mcp-echo` tool binary path |
| `KX_DEMO_BODY_PATH` | `kx serve` | override the `exec-demo` sandboxed body binary path |
| `KX_VERSION` / `KX_INSTALL_DIR` | installer | release tag / install directory |

## Blueprints

A **Blueprint** is a reusable, parameterized workflow published by handle. The
server validates your typed inputs, compiles the workflow to a step DAG, and runs
it with the full durability guarantee. Five ship with `kx serve`:

| Handle | What it runs | Inputs | Available |
|---|---|---|---|
| `kx/recipes/echo` | one deterministic step | `topic` (str) | always |
| `kx/recipes/fanout-demo` | a 5-step fan-out → gather DAG | — | always |
| `kx/recipes/exec-demo` | a real sandboxed process step | — | when the demo body binary is present |
| `kx/recipes/chat` | one LLM completion over your model | `prompt` (str) | inference build + `KX_SERVE_MODEL_GGUF` |
| `kx/recipes/react` | the live ReAct agent loop with tools | `instruction` (str) · `max_turns` (1–8, default 8) · `max_tool_calls` (< max_turns, default 6) | inference build + model + tool |

List what your gateway offers from any surface: the console's **Blueprints**
section, `client.listRecipes()` (SDKs), or the connect-time catalog.

**Author your own** — workflows are composed from pure building blocks
(map-reduce, fan-out/gather, critic-gated retry, ReAct tool loops, image
batch-describe) plus a fail-closed prompt-template engine, then compiled and
submitted like any blueprint. A complete, runnable walkthrough:

```bash
cargo run -p kx-workflow --example author_a_workflow
```

## Datasets & RAG

Durable, content-addressed document corpora with vector search — ground your
agents in your data (`hnsw` builds; included in the prebuilt binary):

- **Ingest** documents with client-supplied embedding vectors (FFI-free, works
  with any embedder you run) — or let an inference build embed server-side.
- **Search** by similarity; results carry exact content hashes, so anything a
  run consumes is pinned to the exact bytes it read.
- Surfaces: the console's **Datasets** section, the SDK `datasets` module
  (`ListDatasets` / `IngestDocuments` / `QueryDataset`).

## SDKs

Both SDKs speak the same wire as the CLI and console — every capability above is
callable from code, with identities always server-derived.

**Python** — `pip install kortecx` (`pip install 'kortecx[ws]'` for live event
streaming):

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151") as kx:
    result = kx.invoke("kx/recipes/echo", {"topic": "hello"}, wait=True)
    print(result.text)
```

Async twin (`AsyncKxClient`), typed errors with stable codes, run handles with
`.projection(at_seq=…)` / `.events(follow=True)` / `.content(ref)`, plus wrappers
for runs, react turns, replans, capture records, datasets, teams, and grants.

**TypeScript/JavaScript** — `npm install @kortecx/sdk` (node + browser entry
points; `npm install ws` for node live-tail):

```ts
import { KxClient } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151");
const result = await kx.invoke("kx/recipes/echo", { topic: "hello" }, { wait: true });
console.log(result.text);
```

## Security defaults

Closed by default, opened explicitly: the server answers **nobody** until you
pass `--dev-allow-local` (loopback-only) or bearer tokens; every listener binds
loopback unless you say otherwise; browser access is **deny-by-default** CORS
(the embedded console auto-grants only its own loopback origin — never a
wildcard); bearer tokens are never persisted by the console or SDKs beyond
memory; all run identities are derived server-side; and any step that touches
the world runs under an explicit grant checked by exact equality — a model can
propose an action, but only the runtime's checks can let it happen.

## Run in Docker

The runtime ships as a small FFI-free container image; durability is proven
*through* the container:

```bash
just docker-smoke            # build + reproduce the canonical digest in-container
docker compose up --build    # the server stack on named volumes (dev token: kx-dev-token)

kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait \
    --endpoint http://127.0.0.1:50151 --token kx-dev-token
docker compose restart kx    # journal + content persist across restarts
```

## Production notes

- **TLS**: in-binary TLS covers the gRPC listener (`--tls-cert/--tls-key`); the
  WebSocket bridge and web console are loopback plaintext — front them with a
  TLS proxy for remote browsers.
- **Scale**: single-system by default (one journal writer, ~18k commits/sec
  ceiling); the same workflows run unchanged when distributed deployment lands.
- **Inference**: one model, single-stream decoding per server today.
- **Observability**: `kx health`, the live event stream, the audit log, and the
  console's Activity/DevTools surfaces.
- **Versions**: pre-1.0 — pin a release tag (`KX_VERSION`) for anything you keep.

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md) for the
build/test/gate path and [GLOSSARY.md](GLOSSARY.md) for the vocabulary; the
design notes live in [`docs/`](docs/). Please open an issue to discuss
substantial changes before sending a pull request.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

## Links

- **Website:** [kortecx.com](https://kortecx.com)
- **Issues:** [github.com/Kortecx/kortecx/issues](https://github.com/Kortecx/kortecx/issues)
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)
- **CI:** [Actions tab](https://github.com/Kortecx/kortecx/actions) — both the full
  gate and the real-GGUF smoke job must pass on every PR.
