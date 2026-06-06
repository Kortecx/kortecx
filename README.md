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
[![Status](https://img.shields.io/badge/status-early%20development-yellow.svg)](#status--roadmap)

kortecx is an **execution kernel for AI agents** — a runtime that turns clever
agent scripts into production software you can trust with real work. It dispatches
steps across workers, survives crashes by replaying from a durable log, and
guarantees a step that touches the world takes effect **exactly once**. Not a
model, not a database, not a chat app: the execution kernel beneath them.

```bash
cargo install --path crates/kx-cli      # the `kx` binary — Rust only, no C++ toolchain
kx run    --journal /tmp/kx.db --content /tmp/kx-content      # → a6b5c679… (8/8 committed)
```

---

## Contents

- [The gap we're closing](#the-gap-were-closing)
- [Prerequisites](#prerequisites)
- [Install & quick start](#install--quick-start) — prove exactly-once in 60 seconds
- [Getting started](#getting-started) — run the runtime as a server, like a function
- [Commands](#commands) — the full `kx` reference
- [Recipes](#recipes) — reusable agentic workflows
- [Local LLM inference](#local-llm-inference) — bring your own GGUF model
- [How it works](#how-it-works) — the architecture in one read
- [Extending](#extending) — bring your own journal, store, broker, backend
- [Production notes & known limitations](#production-notes--known-limitations)
- [Status & roadmap](#status--roadmap)
- [Contributing](#contributing) · [License](#license) · [Links](#links)

---

## The gap we're closing

AI agents work in the demo and fall over in production. The gap is **reliable
orchestration** — dispatching tasks across workers, retrying on failure, surviving
crashes, and never re-running a step that already touched the outside world.

Agents are non-deterministic and they act on the world — they call models, hit
APIs, move money, change state. Run that at scale and the failure modes compound:
a retry double-charges a customer, a crash loses half a job, a redistributed task
silently runs twice. Today every team rebuilds the same fragile glue to cope, and
most never get past the demo. kortecx makes reliability a property of the runtime:

- **Trustable.** A step that touches the world takes effect exactly once. Crashes,
  retries, and redistribution never double-apply or silently drop work.
- **Durable & resumable.** The single source of truth is an append-only log;
  workflows survive process death and resume from committed work, never starting
  over or stalling. *This is the headline guarantee.*
- **Scales without a rewrite.** The same workflow runs on a laptop or across a
  fleet, with the same guarantees. Scale is a deployment choice, not a re-architecture.
- **No infrastructure tax.** Durability, recovery, and coordination are built in,
  so teams ship agents instead of the plumbing beneath them.
- **Bring your own data + tools.** Read from any store, call any API or MCP tool —
  the runtime records what happened, captures the result, and serves it durably.
- **Reusable, not rebuilt.** Actions, workflows, and tools live in a sharable
  catalog: publish a guaranteed action once, reuse it with new parameters.

The core guarantee — *exactly once, even across crashes* — is something you can
verify for yourself in the [quick start](#install--quick-start) below, and every
change to the project is gated by an automated suite that crashes live workflows
and checks that no work is ever lost, duplicated, or double-applied.

## Prerequisites

kortecx has a **two-tier** setup. Most of the runtime needs only Rust.

| Tier | You get | You need |
|---|---|---|
| **Tier 0 — the runtime** *(required)* | `kx run` / `replay` / `serve`, the durability demo, recipes, the full CLI & gateway | **Rust 1.94.0+** only — **no C++ toolchain**. The `kx` binary is FFI-free. |
| **Tier 1 — local LLM inference** *(optional)* | real on-device model inference via llama.cpp | a **C++ toolchain** (CMake, clang/libclang, a C++ compiler) + the `llama.cpp` submodule + a **GGUF** model |

[Rust](https://rustup.rs) honors the pinned toolchain in `rust-toolchain.toml`
automatically. Run **`just doctor`** ([`just`](https://github.com/casey/just)) for
a tiered preflight that checks both tiers and prints the exact install command for
anything missing on your OS.

> Tier 1 install hints — macOS: `xcode-select --install && brew install cmake`;
> Debian/Ubuntu: `sudo apt-get install -y cmake clang libclang-dev build-essential`.
> Then `just setup-inference`.

## Install & quick start

Install the `kx` binary (Rust only — no C++ toolchain):

```bash
git clone https://github.com/Kortecx/kortecx.git && cd kortecx
just setup            # installs `kx` (or: cargo install --path crates/kx-cli)
```

Now **prove exactly-once end to end** — run the canonical demo workflow, crash it
mid-commit, and replay. The digest is identical across the clean run and the
crash-then-replay run:

```bash
# 1. Run the demo to completion, capturing its deterministic digest.
kx run    --journal /tmp/kx.db --content /tmp/kx-content
#    → a6b5c67939f14bfcbd125f7461b2bd0e481f6ee2fc98c1ab638730e2d2ace2e9 (8/8 committed)

# 2. Start fresh, but hard-abort right after a side effect commits.
rm -f /tmp/kx.db; rm -rf /tmp/kx-content
kx run    --journal /tmp/kx.db --content /tmp/kx-content --crash-at post-commit-vtc

# 3. Recover from the journal and finish the run.
kx replay --journal /tmp/kx.db --content /tmp/kx-content
#    → same digest — the crashed step was re-read, not re-run.
```

Same digest = the exactly-once property, demonstrated. (`just verify-quickstart`
runs exactly this and asserts the digest, so these docs can't silently drift.)

## Getting started

The same engine runs as a **server**, so an agent can call the runtime like a
function. In one terminal, start it on loopback with dev auth:

```bash
kx serve --journal /tmp/kx.db --content /tmp/kx-content --dev-allow-local
#    gRPC on 127.0.0.1:50151 · live-event WebSocket on 127.0.0.1:50152
```

In another terminal, **submit a run and wait for the committed result**:

```bash
# A built-in pure demo run (the lowest-level entry point).
kx submit --demo --wait

# Invoke a PUBLISHED recipe by handle, bound to JSON args — run-to-result.
kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait
```

**Inspect** any run — its DAG, a committed result, and its event stream:

```bash
kx projection --instance <instance-id>                       # the run as a DAG of Mote states
kx content    --ref <content-ref> --instance <instance-id>   # fetch a committed result (raw bytes)
kx events     --instance <instance-id> --follow              # live-tail the run's event deltas
```

Every run can write an **audit trail** — an off-the-truth-path JSONL record of the
run lifecycle that never changes the digest:

```bash
kx run --journal /tmp/kx.db --content /tmp/kx-content --audit-log /tmp/audit.jsonl
```

Auth is **deny-all by default**: `--dev-allow-local` trusts loopback only; for
real principals use `--auth-token <token>=<party>` (or `--auth-token-file`), and
pass `--token`/`--token-file` on the client. Identity is always derived
server-side — never asserted by the client.

## Run in Docker

The `kx` binary is FFI-free, so the runtime ships as a small container image — no
C++ toolchain, no CUDA, no model baked in. The durability guarantee is proven
*through* the container, not just asserted:

```bash
# Build the FFI-free image + reproduce the canonical digest IN-CONTAINER
# (clean run · crash-then-replay over a persisted volume · read-only rootfs):
just docker-smoke

# Or bring up the server stack — embedded coordinator + worker + gateway — with the
# journal & content on named volumes that survive a restart:
docker compose up --build
```

With the stack up, drive it from the host (the compose dev token is `kx-dev-token`):

```bash
kx submit --demo --wait --endpoint http://127.0.0.1:50151 --token kx-dev-token
kx invoke kx/recipes/echo --args '{"topic":"durable agents"}' --wait \
    --endpoint http://127.0.0.1:50151 --token kx-dev-token

docker compose restart kx     # journal + content persist on the named volumes
docker compose down           # SIGTERM → graceful drain, not a hard kill
```

**Images.** `Dockerfile` builds the FFI-free runtime (`kortecx/kx:dev`); the
container runs as a **non-root** user (uid 10001), keeps durable state under
`/var/lib/kortecx/{journal,content,catalog}`, and is `--read-only`-rootfs
compatible (only the mounted volumes + `/tmp` are writable). `Dockerfile.inference`
adds the CPU llama.cpp link + a `kx-generate` example for a real CPU inference run.

**Auth + TLS on a published port.** A non-loopback bind refuses `--dev-allow-local`, so
the compose uses **bearer-token auth** (a Docker secret). Bearer-over-plaintext travels
in cleartext, so enable **in-binary TLS** with `kx serve --tls-cert <pem> --tls-key
<pem>` (rustls) and dial it with `kx … --endpoint https://… --tls-ca <pem>` (or a
public CA via the OS trust store) — or front with a TLS reverse proxy. Replace the dev
tokens in `deploy/secrets/` for anything real.

**GPU posture.** OSS GPU inference today is **Metal, on an Apple host** — not in a
Linux container (Metal is macOS-host-only). NVIDIA **CUDA inference is cloud-tier**
(decision D28): `Dockerfile.cuda` is a *documented seam* (the intended image shape +
an `nvidia-smi` detection hook), not a buildable OSS image; multi-tenant
GPU-batched serving lives in the cloud offering.

## Commands

The `kx` CLI is one binary. `run`/`replay`/`digest` drive the engine locally;
`serve` hosts the gateway; the rest are gRPC clients of a running gateway.

| Command | What it does | Key flags |
|---|---|---|
| `kx run` | Drive the canonical demo workflow from scratch | `--journal` `--content` · `--crash-at <pt>` · `--checkpoint-every N` · `--audit-log <path>` · `--json` |
| `kx replay` | Recover an existing journal and finish the run | `--journal` `--content` · `--audit-log` · `--json` |
| `kx digest` | Print the projection digest of a journal | `--journal` `--content` · `--json` |
| `kx serve` | Host the embedded single-system gateway | `--journal` `--content` · `--listen` *(default `127.0.0.1:50151`)* · `--ws-listen` *(default `:50152`)* · `--dev-allow-local` · `--auth-token <t>=<party>` · `--auth-token-file` · `--tls-cert <path> --tls-key <path>` *(in-binary TLS)* · `--max-lease N` · `--catalog-dir` |
| `kx invoke <handle>` | Bind a published recipe to JSON args and run it | `--args <json>` / `--args-file` · `--wait` · `--timeout-secs N` · `--out <file>` |
| `kx submit --demo` | Submit a built-in pure demo run | `--wait` · `--timeout-secs N` · `--out` |
| `kx projection` | Render a run as a DAG of Mote states | `--instance <id>` · `--at-seq N` |
| `kx content` | Fetch a committed result by ref (binary-safe) | `--ref <r>` · `--instance <id>` · `--out <file>` |
| `kx events` | Print / live-tail a run's event deltas | `--instance <id>` · `--since N` · `--follow` |
| `kx signatures` | Browse / fetch / register catalog task signatures | `list` · `get --id <id>` · `register --manifest-file <path>` |

Client verbs share `--endpoint <url>` *(default `http://127.0.0.1:50151`)*,
`--token <t>` / `--token-file <p>`, and `--json`.

**Exit codes:** `0` success · `2` usage/config error · `3` `--wait` timed out (the
run is still in progress and resumable) · `1` everything else (RPC, IO, a failed
Mote). `kx --help` and `kx help <command>` print usage.

## Recipes

A **recipe** is a reusable, parameterized workflow that compiles to a Mote DAG.
Five are shipped (all deterministic, statically shaped), composable from pure
building blocks plus a fail-closed prompt-template engine:

| Recipe | Shape |
|---|---|
| `map_reduce` | N mappers → one pure reduce |
| `fan_out_gather` | N parallel non-deterministic workers → one pure gather |
| `retry_until_critic` | N independent attempts, each critic-gated → one selector (bounded best-of-N) |
| `react_tool_loop` | one ReAct turn: reason → act (tool) → observe |
| `image_batch_describe_reduce` | one describe step per image → one pure reduce |

Author and run your own end to end — author a workflow → compile to a Mote DAG →
run → fold the journal:

```bash
cargo run -p kx-workflow --example author_a_workflow
```

## Local LLM inference

Inference is **Tier 1** (opt-in). Set up the native backend and fetch a tiny demo
model, then point any GGUF through the examples:

```bash
just setup-inference          # init the llama.cpp submodule + build the FFI link (needs a C++ toolchain)
just fetch-demo-model         # download a ~1.2 MB GGUF to target/models/ (SHA-256 verified)

cargo run -p kx-llamacpp --example generate -- target/models/stories260K.gguf "Once upon a time"
cargo run -p kx-llamacpp --example chat     -- /path/to/your-model.gguf
cargo run -p kx-llamacpp --example embed    -- /path/to/your-model.gguf "embed this"
```

In the runtime, model inference is a trait seam (`InferenceBackend`); the llama.cpp
backend is one implementation, and you point it at a GGUF file by path.

## How it works

kortecx is an **execution kernel**. The unit of work is a **Mote** — one step,
content-addressed by its definition + inputs, so identical work has an identical
identity. The single source of truth is an append-only **journal**: the runtime
never holds authoritative state in memory, it *appends facts* (`Proposed`,
`Committed`, `Failed`, `Repudiated`, `EffectStaged`, …) to the log. All live state
is a **projection** — a pure *fold* of the journal, re-derived from scratch on
restart. A crash loses no truth, because the truth is the log, and recovery is
just folding it again. A step that changes the world is driven through a **commit
protocol** that records intent before it acts, so the effect lands exactly once.

```
  submit ─► register run (immutable instance id)
         ─► submit Mote ─► REFUSAL GATE (refuse unsafe constructions up front)
         ─► journal.append(Proposed)
                 │
   scheduler ◄───┴── reads the ready set from the projection fold
         │           (a Mote is ready when its parents are committed)
         ▼
   executor ── dispatches under a commit protocol:
         │       • IdempotentByConstruction → effect → append(Committed)
         │       • StageThenCommit          → append(EffectStaged) → effect → append(Committed)
         │       • ValidateThenCommit       → effect → critic verdict → append(Committed|Repudiated)
         │     (world effects go ONLY through the CapabilityBroker)
         ▼
   journal ── append(Committed)  ─►  projection folds it  ─►  consumers unblock
```

**Recovery** runs the same machinery in reverse: on restart the runtime re-folds
the journal; an `EffectStaged` with no matching `Committed` tells an oracle a crash
landed mid-effect, and it decides whether re-dispatch is safe or the effect must be
quarantined — that is how exactly-once survives a crash inside a world-mutating step.

**The crates** — 39 in a clean layered DAG (no cycles). The foundation is a narrow
waist almost everything depends on; the engine and the optional distributed layer
stack on top:

- **Waist (the guarantee path):** `kx-mote` → `kx-content` → `kx-journal` →
  `kx-warrant` → `kx-projection`. The load-bearing invariants live here.
- **Engine:** `kx-capability` (the single door to world effects), `kx-scheduler`,
  `kx-executor` (lifecycle + commit protocols + recovery), `kx-inference`,
  `kx-critic`, `kx-runtime` (the single-node engine + `run`/`replay`/`digest`).
- **Reach (v0.1.0):** `kx-gateway`/`kx-gateway-core` (the gRPC server),
  `kx-cli` (the `kx` binary), `kx-invoke` (recipe → guaranteed run),
  `kx-workflow` (recipes + prompt templating), `kx-catalog` (sharable signatures),
  `kx-fleet` (teams), `kx-audit` (off-path audit trail).
- **Distributed (optional):** `kx-coordinator` (sole journal writer) +
  `kx-worker` + `kx-proto` — same guarantees, wiring on the same seams, not a rewrite.
- **Forward seams (off the guarantee path):** `kx-capture`, `kx-dataset`,
  `kx-memoizer`, `kx-tiering`, `kx-normalizer`.

See [GLOSSARY.md](GLOSSARY.md) for the vocabulary, and the doc-comments on the core
types for the deeper *why*. **New to the code? Start at a leaf or an example, not
the executor.**

## Extending

Deployment and customization happen at **trait seams** — the same trait is
implemented one way locally and another for a hosted/distributed deployment, so
distribution and cloud are new implementations, not a rewrite of the engine.

| Seam | Trait | Defined in | Abstracts |
|---|---|---|---|
| Journal | `Journal` | `kx-journal/src/lib.rs` | the append-only log of facts (local: SQLite) |
| Content store | `ContentStore` | `kx-content/src/lib.rs` | content-addressed bytes (local: filesystem) |
| Capability broker | `CapabilityBroker` | `kx-capability/src/broker.rs` | the single door to world effects + idempotency |
| Inference backend | `InferenceBackend` | `kx-inference/src/backend.rs` | model inference (local: llama.cpp) |
| Resource manager | `ResourceManager` | `kx-executor/src/resource_manager.rs` | admission/slots for dispatch |
| Secret store | `SecretStore` | `kx-mcp/src/secret_store.rs` | secret resolution for capabilities (never journaled) |
| Worker registry | `WorkerRegistry` | `kx-coordinator/src/registry.rs` | worker liveness (distributed only) |

Implement a trait, swap it in at construction — the guarantee machinery is unchanged.

## Production notes & known limitations

kortecx is in early development; the durability spine is real and tested, but the
reach surface is young. We name the boundaries plainly rather than hide them:

- **Transport: in-binary TLS or plaintext.** `kx serve --tls-cert/--tls-key` serves
  rustls TLS on the gRPC listener (clients dial `https://…` with `--tls-ca` for a
  self-signed cert, or the OS trust store for a public CA); the bearer-over-plaintext
  path is still warned about. The **WebSocket bridge stays plaintext** for now — front
  it (or the whole server) with a TLS proxy if you need `wss`. **mTLS** client-cert auth
  is a follow-on.
- **Auth is bearer-token + deny-all.** Identity is server-derived; the
  `PrincipalResolver` seam is where OIDC/mTLS plug in later. There is no
  multi-tenant isolation or per-tenant quota yet.
- **Single-system scale.** One process is the sole journal writer (SQLite); a
  25k-Mote journal folds sub-linearly on cold recovery (gated in CI), but
  multi-node orchestration is the distributed layer / hosted offering, not the
  default single-system runtime.
- **Inference is single-stream** (N=1, serialized) and models are referenced by
  path — there is no model registry or auto-download in the runtime yet.
- **Observability** today is the audit log + the event stream; a metrics/OTel
  export seam is on the roadmap.

Interfaces will change before 1.0 — **pin a commit** if you build on it now.

## Status & roadmap

**Early development, built in the open.** Today (v0.1.0): a durable single-system
runtime with exactly-once world effects and crash recovery, a gateway server, the
unified `kx` CLI, a recipe library + prompt templating, an audit trail, and a
live event stream. Next: TypeScript & Python client SDKs over gRPC, a dashboard,
audio multi-modal inference, and an opt-in cluster layer for managed multi-node.

## Contributing

Contributions are welcome. Start with [CONTRIBUTING.md](CONTRIBUTING.md) for the
build/test/gate path and where to begin, the [How it works](#how-it-works) section
for the architecture, and [GLOSSARY.md](GLOSSARY.md) for the vocabulary. Please
open an issue to discuss substantial changes before sending a pull request.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

## Links

- **Website:** [kortecx.com](https://kortecx.com)
- **Issues:** [github.com/Kortecx/kortecx/issues](https://github.com/Kortecx/kortecx/issues)
- **Changelog:** [CHANGELOG.md](CHANGELOG.md)
- **CI:** [Actions tab](https://github.com/Kortecx/kortecx/actions) — both the full
  gate and the real-GGUF smoke job must pass on every PR.
