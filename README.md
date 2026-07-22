# kortecx

> The durable runtime for AI agents.
> **Knowledge → Intelligence.**

🌐 **[kortecx.com](https://kortecx.com)** &nbsp;·&nbsp; built in the open at [Kortecx/kortecx](https://github.com/Kortecx/kortecx)

[![CI](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![MSRV](https://img.shields.io/badge/MSRV-1.94.0-orange.svg)](rust-toolchain.toml)
[![Rust Edition](https://img.shields.io/badge/Rust-2021-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/index.html)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue.svg)](#)
[![Status](https://img.shields.io/badge/status-early%20development-yellow.svg)](#)

kortecx runs AI agents you can trust with real work. One small binary gives you
**durable, exactly-once agentic execution** — live agent loops that plan,
re-plan, self-check, and call tools; reusable **Blueprints** and **Chains**;
portable, shareable **Apps**; **RAG datasets** and durable **memory**; **local
LLM inference**; a built-in **web console**; and **Python/TypeScript SDKs** — all
over an append-only journal that survives crashes and never runs a world-touching
step twice.

```bash
curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
kx serve --dev-allow-local          # zero-config — the journal, content store, and catalog auto-resolve under ~/.kortecx
# → gRPC 127.0.0.1:50151 · events ws://127.0.0.1:50152 · web console http://127.0.0.1:8888
```

---

- [What you get](#what-you-get) · [How it works](#how-it-works)
- [Install](#install) · [Prerequisites](#prerequisites)
- [Quick start: prove exactly-once](#quick-start-prove-exactly-once)
- [Start the runtime locally](#start-the-runtime-locally) — serve, run blueprints, chat, ReAct with tools
- [The web console](#the-web-console)
- [CLI reference](#cli-reference) — every command, flag, and environment variable
- [Blueprints](#blueprints) · [Chains](#chains) · [Apps](#apps) · [Datasets & RAG](#datasets--rag) · [SDKs](#sdks)
- [Security defaults](#security-defaults) · [Run in Docker](#run-in-docker)
- [Production notes](#production-notes)
- [Contributing](#contributing) · [License](#license) · [Links](#links)

---

## What you get

| Capability | What it does | Where |
|---|---|---|
| **Apps** *(the shareable unit)* | Package a whole agent — its blueprint, prompts, skills, memory rail, and tool/connection/dataset references — as one durable `kortecx.app/v1` envelope, **plus a project** (a tree of files the model authors into a content-addressed branch) you edit in the console IDE. Run, clone, export, and import the envelope elsewhere; it carries no authority, so every warrant re-resolves against the importer's own grants. **The bundle carries the envelope + its content closure, not the project tree.** | `kx app` · SDKs · console Apps |
| **Exactly-once agentic runs** | Every step (a *Mote*) commits durably to an append-only journal; crashes replay from committed work — a step that touched the world is re-read, never re-run | CLI · SDKs · console |
| **The live agent loop** | Models **plan** topology, **re-plan** on failure, pass **critic** gates, and run **ReAct turns with real MCP tools** — all inside `kx serve`, all crash-safe | `kx/recipes/react` + the console Chat |
| **Blueprints** | Reusable, parameterized workflows published by handle — pick one, fill its typed inputs, run it, watch the live DAG | CLI `kx invoke` · SDKs · console |
| **Chains & Swarms** | Compose published task handles into a DAG with a small string DSL (`>` `&` `\|` `[ ]`), or run a multi-agent pattern (swarm · supervisor · consensus) without hand-writing it — both lower to the same compile + warrant path as a Blueprint | `kx chain` · `kx swarm` · SDKs |
| **Local LLM inference** | Bring any fit GGUF model; on-device llama.cpp (Metal/CPU) or an auto-detected Ollama daemon drives chat + the agent loop — no API keys, no egress | `kx serve` (inference / serve-engine build) |
| **Datasets & RAG** | Ingest documents, search by vector similarity, ground agent runs — durable, content-addressed corpora | CLI/SDK/console Datasets |
| **Durable memory** | Agents remember facts and recall them across runs — server-embedded, scoped to the caller's principal, recalled by similarity | `kx memory` · SDKs |
| **Live events & time-travel** | Stream every state change as it commits; scrub any run back to any point in its history | `kx events --follow` · console Activity |
| **Run capture (Morphic)** | Every serve-path run's actions are captured to a durable sidecar — your agents' exhaust becomes queryable data | SDKs (`ListCaptureRecords`) |
| **Teams & grants** | Durable membership + asset grants with resolved-warrant views | console Systems · SDKs |
| **Audit trail** | An off-the-truth-path JSONL record of the run lifecycle | `kx run --audit-log` |
| **Observability & cost** | Per-mote execution telemetry, a per-model token rollup, a per-run local spend estimate, a terminal-failure alerts inbox, and an opt-in Prometheus `/metrics` endpoint — all audit/display-only, never truth | `kx telemetry` · `kx cost` · `kx alerts` · console |
| **The web console** | All of the above in a browser — served by `kx` itself, zero extra setup | `http://127.0.0.1:8888` |

Every capability is reachable from the **CLI**, the **Python and TypeScript
SDKs**, and the **web console** — same wire, same guarantees.

## How it works

The atomic unit of execution is the **Mote** — a durably-recorded *record of an
attempt to effect something*. Once a Mote commits, it is a fact in an append-only
**journal**, and that fact is never re-run: a step that touched the world is
re-read on recovery, never re-executed. This is what makes exactly-once hold
across crashes, retries, and restarts.

The three invariants everything else is built on:

1. **The journal is the single source of truth.** The scheduler, executor, and
   recovery coordinate *only* through committed journal facts — never through
   direct messaging. That is why the single-node → distributed step is wiring on
   the same seams, not a rewrite.
2. **The journal is an append-only log; the graph is a projection.** Execution is
   a DAG of Motes, but the live state (each Mote's status, the ready set, the
   dependency index) is a **pure fold** of the log — re-derived on restart, never
   stored as a durable mutable graph.
3. **The model proposes; the runtime enforces.** A model may propose a tool to
   call, a role to assume, or content to emit; the runtime enforces capability
   checks and **exact-equality** identity matching (fuzzy search may *discover*
   candidates, but a world-mutating action always commits an exact reference).

The code is a clean layered DAG — leaf types (`kx-mote`, `kx-journal`,
`kx-content`) fold up through the projection + executor kernel into the
coordinator/worker/gateway, and the CLI, SDKs, and console are all clients of the
same gateway. The `Journal` and `ContentStore` seams are **traits**, so scaling
out swaps an implementation without touching the engine. See
[GLOSSARY.md](GLOSSARY.md) for each term and the crate it lives in.

## Install

```bash
# Prebuilt binary (Linux x86_64/arm64, macOS arm64) — SHA-256 verified, no sudo,
# installs to ~/.local/bin. The prebuilt ships the web console + Datasets, and
# serves local models via a running Ollama daemon out of the box (no C++ toolchain).
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
cargo install --path crates/kx-cli --features serve-engine,hnsw  # + serve local models via Ollama (still no C++)
cargo install --path crates/kx-cli --features inference,hnsw  # + in-process llama.cpp inference (needs a C++ toolchain)
just console-build                                          # + the embedded web console — builds console,hnsw,serve-engine,hosted-apps (exactly the prebuilt; needs node 22; repo checkout only)
```

> The web console is embedded at **compile time**, so `--features console` needs
> the built SPA (`just console-dist`) — use the prebuilt binary if you don't want
> node. Plain `cargo install` never needs node or C++.

## Prerequisites

| Tier | You get | You need |
|---|---|---|
| **Tier 0 — the runtime (prebuilt)** | the runtime + the web console + Datasets + **Apps end-to-end** — scheduled Apps (agentic scaffold, tools, skills, connections, cron triggers) **and** hosted (web-experience) Apps served on a loopback port — serving local models via a running **Ollama** daemon (zero toolchain) | nothing (prebuilt) or **Rust 1.94+** (source); **Ollama** for local inference; **Node/npm** on the host to serve a hosted App |
| **Tier 1 — in-process llama.cpp** | self-contained on-device inference (no daemon) + multi-modal / vision | a **C++ toolchain** (CMake, clang/libclang) + a **GGUF** model |

Both engines are co-equal first-class backends — see
[Local inference engines](docs/site/docs/local-inference-engines.md) for the
positioning (Ollama = quick/easy; llama.cpp = performance / parallel / multi-modal)
and the capability matrix.

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
bridge, and (prebuilt binaries) the web console. It's **zero-config** — you pass
only the auth posture; the journal, content store, and catalog auto-resolve under
`~/.kortecx` (override the base with `KX_DATA_DIR`, or pin individual paths with
`--journal`/`--content`) and persist across restarts:

```bash
kx serve --dev-allow-local
#    gRPC on 127.0.0.1:50151 · events on ws://127.0.0.1:50152
#    web console at http://127.0.0.1:8888  ← open this in your browser
#    (a startup banner prints every resolved path + endpoint)
```

**2. Run your first blueprints** (another terminal):

```bash
# A single-step echo — the canonical hello-world (typed input: topic).
kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait

# A real multi-node DAG, model-free: root → 3 children → gather (5 steps, all committed).
kx invoke kx/recipes/passthrough-dag --args '{}' --wait
```

**3. Inspect anything** — the DAG, a committed result, the live event stream:

```bash
kx projection --instance <instance-id>                       # the run as a DAG of step states
kx projection --instance <instance-id> --at-seq 3            # …time-traveled to any point
kx content    --ref <content-ref> --instance <instance-id>   # a committed result (raw bytes)
kx events     --instance <instance-id> --follow              # live-tail the run's events
```

**4. Add a local model.** Two paths — see
[Local inference engines](docs/site/docs/local-inference-engines.md):

- **Ollama (zero-friction, no C++).** Build with `--features serve-engine,hnsw`,
  [install Ollama](https://ollama.com), and `kx serve` auto-detects it on the
  loopback port:

  ```bash
  ollama pull gemma3:12b
  kx serve --dev-allow-local
  ```

- **llama.cpp (self-contained).** Build with `--features inference,hnsw`, download a
  **fit** GGUF, and point the server at it. The runtime validates fitness at startup:
  a chat template (ChatML), **native tool-calling**, a commercial-friendly license,
  `q4_k_m`/`q8_0`/`f16` quantization, and a context window ≥ 2048. The Qwen3 family
  fits; the 0.6B stand-in below runs on ~0.5 GB:

  ```bash
  curl -fsSL -o qwen3-0.6b-q4_k_m.gguf \
    https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf
  shasum -a 256 qwen3-0.6b-q4_k_m.gguf
  #    → ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a

  KX_SERVE_MODEL_GGUF="$PWD/qwen3-0.6b-q4_k_m.gguf" \
    kx serve --dev-allow-local
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
**`http://127.0.0.1:8888`** — no node, no separate install. Connect to your
gateway endpoint (pre-filled: `http://127.0.0.1:50151`; the bearer token, if you
use one, stays in browser memory and is never stored).

| Section | What you do there |
|---|---|
| **New Chat** *(default)* | a fresh agentic conversation over the runtime — every turn a durable run |
| **Apps** | create an App agentically (**New App** — the model plans and writes its project tree), then browse, run, schedule, and open durable `kortecx.app/v1` Apps in a per-App IDE (files + Monaco editor, lineage graph, chat) |
| **Workflows** | browse Blueprints and trigger a run; your run history and the self-correction (re-plan / ReAct) trails |
| **Context** | reusable instruction/file bundles and RAG **Datasets** |
| **Integrations** | tools, external MCP **connections**, event **triggers**, and **secrets** |
| **Models** | the models this gateway serves — pick the default |
| **Security** | an App's resolved capability manifest — reach, capability ceiling, and model route |
| **Settings** | connection profile + console preferences |

**Activity** is the navbar drawer — the live event feed, run metrics, and
**time-travel** (scrub any run's history) from any screen. Pending **HITL approvals**
for world-mutating actions live in the navbar **bell** (a cross-App inbox), not in the
Apps section. Plus **⌘K** jumps anywhere, and the **DevTools dock** (navbar toggle) tails
live events and gateway health. Blueprints, Datasets, and Branches also keep their own deep-linkable
routes. Override the console address with `--console-listen <addr:port>`
(loopback only) or turn it off with `--no-console`. For a remote browser,
static-host `ui/dist` and grant its origin with `--cors-origin`.

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
kx serve --dev-allow-local [flags]     # zero-config: journal/content/catalog auto-resolve under ~/.kortecx
```

| Flag | Default | Meaning |
|---|---|---|
| `--journal <path>` / `--content <dir>` | under `~/.kortecx` | durable journal + content store (zero-config; override the base with `KX_DATA_DIR`, or pin either explicitly) |
| `--catalog-dir <dir>` | beside the journal | durable catalog + sidecars (blueprints, signatures, teams, telemetry, capture) |
| `--listen <addr:port>` | `127.0.0.1:50151` | the gRPC + gRPC-web endpoint |
| `--ws-listen <addr:port>` | `127.0.0.1:50152` | the live-event WebSocket bridge |
| `--console-listen <addr:port>` | `127.0.0.1:8888` | the embedded web console (loopback only) |
| `--no-console` | — | disable the web console |
| `--dev-allow-local` | off | dev auth: allow loopback callers (loopback binds only) |
| `--auth-token <token>=<party>` | — | accept a bearer token as a party (repeatable) |
| `--auth-token-file <path>` | — | `token=party` per line (`#` comments) |
| `--cors-origin <origin>` | deny all | allow a browser origin on the gRPC-web shim (repeatable, never a wildcard) |
| `--tls-cert <pem> --tls-key <pem>` | plaintext | in-binary TLS for the gRPC listener |
| `--max-lease <N>` | `16` | embedded-worker lease batch size |
| `--workers <N>` | `1` | embedded-worker pool size (`>1` runs Pure/IO/tool Motes concurrently) |
| `--content-max-bytes <N>` | built-in cap | the `PutContent` payload cap (fail-closed) |
| `--metrics-listen <addr:port>` | off | opt-in Prometheus `/metrics` endpoint (RED metrics; FFI-free) |
| `--webhook-listen <addr:port>` | off | opt-in inbound webhook surface for event triggers (per-trigger HMAC/bearer) |
| `--audit-log <path>` | off | best-effort JSONL audit trail of the run lifecycle |

**Auth is deny-all by default** — pass `--dev-allow-local` (local development) or
bearer tokens. A bare `kx serve` with no auth posture fails fast with a hint; it
never opens an unauthenticated server.

### Client commands

Every client command takes the **shared flags**: `--endpoint <url>` (default
`http://127.0.0.1:50151`) · `--token <t>` / `--token-file <p>` (bearer auth;
prefer the file — `--token` is visible in `ps`) · `--tls-ca <pem>` (for
`https://` endpoints) · `--json` (machine-readable output).

Run `kx help <command>` for the full per-command usage (flags, subcommands,
examples). The shipped verbs, grouped by purpose:

**Author & run**

| Command | What it does |
|---|---|
| `kx invoke <handle>` | run a published blueprint to a committed result (`--args`/`--args-file` · `--wait` · `--stream` · `--out`) |
| `kx chain run "<dsl>"` | author a DAG from the string-DSL (`>` `&` `\|` `[ ]`) over `--tasks`, then run it (`--emit-blueprint` · `--dry-run`) |
| `kx swarm "<agent>"…` | run a multi-agent pattern (`--pattern swarm`/`supervisor`/`consensus`) without hand-writing the DSL |
| `kx blueprint run\|import` | run a portable blueprint DAG from `--file`, or validate + summarize one offline |
| `kx agent run --goal <text>` | the embeddable agent-runner: a goal → a reasoned answer + the audited action set |
| `kx chat --message <text>` | one chat turn — plain, AUTO-RAG-grounded (`--dataset`), or a bounded agentic turn (`--tools`) |
| `kx app …` | author, run, share, and edit `kortecx.app/v1` Apps — envelope (`new`/`save`/`list`/`get`/`manifest`/`run`/`delete`), portability (`export`/`import`/`clone`), project (`scaffold`/`files`/`cat`/`edit`/`structure`), policy (`lock`/`unlock`) |

**Inspect & observe**

| Command | What it does |
|---|---|
| `kx runs list\|rerun` | durable run history, newest-first; re-run a prior run with edited args (`--set k=v`) |
| `kx projection --instance <hex>` | render a run as a DAG of step states (`--at-seq <N>` time-travels) |
| `kx mote show <instance> <mote>` | display-only Mote definition inspection |
| `kx content get\|put` | fetch a committed result (binary-safe), or upload a blob to the content store |
| `kx events --instance <hex>\|--all` | print or live-tail a run's event deltas, or the global cross-run tail (`--follow`) |
| `kx telemetry list\|summary` | per-mote execution telemetry + the per-model token rollup (audit/display-only) |
| `kx cost <instance>` | a run's local spend estimate |
| `kx capture list` | Morphic captured-action join-key records |
| `kx react\|replan\|rerank list` | the ReAct / re-plan / LLM-rerank self-correction trails (read-only) |
| `kx alerts list` · `kx feedback` | the terminal-failure alerts inbox; submit/list 👍/👎 feedback |

**Catalog, data & memory**

| Command | What it does |
|---|---|
| `kx recipe list\|search` | advisory recipe discovery (scores never authorize — SN-8) |
| `kx signatures list\|get\|register` | the sharable task-signature catalog |
| `kx models list\|load\|offload` | model discovery + local RAM lifecycle |
| `kx datasets list\|ingest\|query` | the RAG data-plane (needs `--features hnsw`) |
| `kx memory add\|recall\|list\|forget\|…` | durable cross-run agent memory (server-embedded, per-principal) |
| `kx context add\|list\|get\|remove` | reusable context bundles (attach via `invoke --context`) |
| `kx branch create\|snapshot\|list\|…` | content-addressed file branches |

**Tools, integrations & config**

| Command | What it does |
|---|---|
| `kx tools list\|score\|discover\|register` | advisory tool discovery + the durable tools registry (scores never authorize) |
| `kx connections add\|list\|test\|remove\|fire` | external MCP connections — dial a connector, fire one tool through the broker |
| `kx skills add\|list\|show\|remove` | the declarative `kortecx.skill/v1` catalog (a grant *wish*, never authority) |
| `kx secrets set\|list\|rm` | the local OS-keychain secret store (write-only values, names-only reads) |
| `kx triggers add\|list\|test\|fire\|rm` | event-ingress triggers (webhook / cron / grpc → a recipe handle `--recipe` **or a saved App handle `--app`**; cron takes a 5-field crontab expression + `--timezone`) |
| `kx approvals list\|grant\|deny` | the HITL pre-action approval gate for world-mutating actions |
| `kx new skill\|connector <name>` | scaffold a skill pack / MCP connector crate offline |
| `kx info` · `kx health` · `kx eval` | non-secret server config · gRPC liveness · agentic evaluation (`run`/`score`) |
| `kx help [command]` · `kx --version` | usage · version |

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
| `KX_DATA_DIR` | `kx serve` | base directory for the zero-config data layout (default `~/.kortecx`) |
| `KX_WORKERS` / `KX_SERVE_WORKER_POOL` | `kx serve` | embedded-worker pool size (same as `--workers`; default `1`) |
| `KX_SERVE_MEMORY` | `kx serve` (inference build) | enable the durable agentic-memory RPCs + the `kx memory` surface |
| `KX_SERVE_MODEL_GGUF` | `kx serve` (inference build) | absolute path to the GGUF model; enables `kx/recipes/chat` + `kx/recipes/react` |
| `KX_N_GPU_LAYERS` | inference | GPU offload layers (Metal/CUDA; default: all that fit) |
| `KX_FLASH_ATTN` | inference | enable flash attention |
| `KX_KV_TYPE` | inference | KV-cache quantization type |
| `KX_N_THREADS` | inference | CPU threads for inference |
| `KX_MCP_ECHO_PATH` | `kx serve` | override the bundled `mcp-echo` tool binary path |
| `KX_VERSION` / `KX_INSTALL_DIR` | installer | release tag / install directory |

## Blueprints

A **Blueprint** is a reusable, parameterized workflow published by handle. The
server validates your typed inputs, compiles the workflow to a step DAG, and runs
it with the full durability guarantee. Four ship with `kx serve`:

| Handle | What it runs | Inputs | Available |
|---|---|---|---|
| `kx/recipes/echo` | one deterministic step (echoes your `topic`) | `topic` (str) | always |
| `kx/recipes/passthrough-dag` | a 5-step fan-out → gather DAG | — | always |
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

## Chains

A **Chain** composes published task handles into a DAG with a small string DSL —
the same expression lowers identically from the CLI and both SDKs, and compiles
through the exact same warrant + durability path as a Blueprint (a chain only
changes how the topology is *authored*). Operators, tightest → loosest: `[ … ]`
grouping · `>` sequential (a data edge, parent → child) · `&` / `|` parallel
merge. A handle that appears twice is the same node, so reuse builds real DAGs.

```bash
# `a` fans out to `b` and `c`, which run in parallel:  a → b, a → c
kx chain run "a > [b & c]" \
  --task a='{"kind":"pure"}' --task b='{"kind":"pure"}' --task c='{"kind":"pure"}' \
  --wait

# lower + validate offline (no gateway) and save the portable blueprint it emits:
kx chain run "[a & b] > c" --tasks tasks.json --dry-run --emit-blueprint chain.json
```

Define each handle's step inline with `--task <name>='{…}'`, all at once with
`--tasks-json '{…}'`, or from a file with `--tasks <file.json>`; a step is
`{"kind":"pure"|"model"|"tool", …}`. **Multi-agent** patterns compose the same
way without hand-writing the DSL — `kx swarm "<agent>"…` (or the SDK `swarm()` /
`supervisor()` / `consensus()` builders) fans N agents to a synthesizer, has a
lead plan → a team execute → integrate, or takes a best-of-N vote. The SDK
surface is `chain(...)` / `Chain` / `Task` (Python) and `chain` / `task` / `seq`
/ `par` / `group` (TypeScript).

## Apps

An **App** is the durable, shareable unit of agentic capability — a
`kortecx.app/v1` envelope that wraps a portable blueprint with by-reference
context, tool, connection, and dataset references, a prompt/rule/skill/memory
rail, and a steering config (model, max turns, max tool calls). Author one from a
blueprint, save it to the caller-scoped catalog, run it server-side, then export
it to share:

```bash
# author an App from a portable blueprint, save it to the catalog, and run it
kx app new my-agent --from-blueprint chain.json --max-turns 4 --output my-agent.app.json
kx app save my-agent.app.json --handle apps/local/my-agent
kx app run apps/local/my-agent --wait

# share it: export a self-contained bundle (envelope + content closure — NOT the
# project tree), then import it on another instance
kx app export apps/local/my-agent --bundle my-agent.kxapp
kx app import my-agent.kxapp
```

An App **carries no authority**: `run` and `import` re-resolve every warrant from
the *caller's own* grants (a model can propose, but only the runtime's checks let
an action happen — SN-8). Connections and secrets never travel in a bundle — the
importer re-registers them by name, so a shared App resolves each operator's own
credentials. Author Apps from code too — the Python `kx.app("…")` and TypeScript
`app("…")` builders (with a `.skill(…)` / `.with_gmail()` rail) — and browse, run,
and open them in the console's **Apps** section.

### An App has a project

Beyond the envelope, an App owns a **project**: a tree of markdown files (README,
`prompts/system.md`, `rules/guardrails.md`, `skills/main.md`, plus goal-specific
extras) that a served model authors for your goal, written into the App's
content-addressed branch. Drive it with `kx app scaffold <handle> --goal "…" --wait`,
or click **New App** in the console — which plans, streams, and writes the tree live.
Browse it with `kx app files` / `kx app cat`, or in the console IDE (file tree + Monaco
editor, an editable lineage graph, and a chat tab). Edits stay in-CAS; the host
filesystem is never written.

> **The project markdown reaches the model at run time.** A rule in `rules/*.md` (or any
> `.md` in the project) rides the App's context rail on every run, so editing
> `rules/guardrails.md` in the IDE changes what the agent does next — up to a total budget
> (`KX_APP_PROJECT_RAIL_BYTES`, 12 KiB), over which the run refuses rather than truncating.
> Non-`.md` files are project docs only.

> **Scaffolding needs a served model.** With no model the scaffold degrades to the base
> file set with generic content. The prebuilt binary satisfies this via a running Ollama
> daemon.

### Run an App on a schedule

An App is a first-class trigger target: `kx triggers add --name nightly --kind cron
--app apps/local/my-agent --schedule "0 9 * * 1-5" --timezone America/New_York`, or the
calendar button on the App card in the console. **Local cron ships in OSS.**

### Two kinds of App

An App is one of two **kinds**, chosen in the console's **New App** form:

- **Scheduled (functional)** — the durable, headless kind above: it wraps a blueprint,
  runs on demand or on a trigger, and is what every `kx app` verb operates on. The
  supported lane.
- **Hosted (experience)** — a real Vite-React or Next.js web project the runtime scaffolds
  into the App's branch and serves on a loopback dev-server port.

Both ship in the prebuilt binary and every `just serve*` recipe. **Serving a hosted App
needs Node/npm on the host** (the supervisor runs `npm install` and a dev server), and the
runtime type-checks the scaffolded project before serving — a project that doesn't compile
fails loudly instead of serving a blank page.

> **Known limits, said plainly.** A curated connector (`kx connections add --provider gmail`)
> points at a connector *binary* (`kx-connector-gmail`, …) the release does **not** package —
> build it from a checkout (`cargo install --path integrations/kx-connector-gmail`) or point
> `--command` at your own MCP server; `kx connections doctor` reports what resolves. The
> `.kxapp` bundle carries the envelope's content closure but **not** the project branch, so a
> hosted App round-trips to an empty shell. A declared dataset's `retrieve@1` grant is
> enforced, but its *use* is persuasion (the model may not call it) — inspect the actual calls
> with `kx react list`. Hosted **Stop** kills the npm wrapper, not always the dev server that
> owns the port, and hosted server state does not survive a gateway restart.

## Datasets & RAG

Durable, content-addressed document corpora with vector search — ground your
agents in your data (`hnsw` builds; included in the prebuilt binary):

- **Ingest** documents with client-supplied embedding vectors (FFI-free, works
  with any embedder you run) — or let a `serve-engine`/`inference` build embed
  server-side (the prebuilt binary can, via a running Ollama embed model).
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
`.projection(at_seq=…)` / `.events(follow=True)` / `.content(ref)`, the
App/Chain/Swarm authoring builders, and wrappers for runs, memory, react/replan/
rerank turns, capture records, telemetry, cost, datasets, teams, and grants.

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
- **Observability**: `kx health`, the live event stream, and the audit log, plus
  per-mote execution telemetry (`kx telemetry`), a per-run local spend estimate
  (`kx cost`), a terminal-failure alerts inbox (`kx alerts`), and an **opt-in
  Prometheus `/metrics`** endpoint (`--metrics-listen`, RED metrics) — all
  audit/display-only, never truth or a digest input. Input-token counts are not
  measured in the OSS backend.
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
