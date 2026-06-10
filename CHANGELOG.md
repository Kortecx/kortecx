# Changelog

All notable changes to kortecx are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). kortecx is in early
development; interfaces may change before 1.0 — pin a commit if you build on it.

## [Unreleased]

### Added

- **Live ReAct substrate in `kx serve` (PR-2d-1, answer-only)** (`crates/kx-toolcall`
  NEW, `crates/kx-journal`, `crates/kx-projection`, `crates/kx-coordinator`,
  `crates/kx-gateway`, `crates/kx-gateway-core`, `crates/kx-model-harness`). The
  harness ReAct loop's substrate now runs LIVE: a `SubmitMoteSpec.react_seed` flag
  (additive, default-false) makes the coordinator swap in a **run-salted** turn-0
  model Mote (`blake3("kx-react-turn" ‖ instance_id ‖ turn)` — server-derived
  identity, collision-free in serve's shared journal) and anchor a durable
  **`ReactRound`** fact (journal schema **v7→v8**, kind 9; off-DAG, never a digest
  input) recording the chain's base prompt, warrant, and budget caps. The
  sole-writer coordinator settles each committed turn by decoding its RAW output
  through the new **`kx-toolcall`** pure leaf (the tool-call authority gate,
  EXTRACTED from `kx-model-harness` so the gateway fence, the coordinator settle,
  and the harness loop share ONE implementation), freezes the branch
  (`Answer`/`Tool`/`DeadLettered`/`Pending`) as a durable fact, advances the chain
  under the fold-re-derived budget (the harness `>=`/tool-then-turn gate,
  line-for-line), and serves the trajectory to the next turn via the F-7 seam in
  transcript order. Crash recovery re-derives the whole chain from committed facts
  alone (the in-flight turn rebuilds to the SAME salted identity — R49; committed
  turns are served, never re-sampled). The gateway's model router gains a
  `react_turn` arm: raw-commit on a normal completion, fail-closed on a malformed
  proposal, and an **answer-only fence** that dead-letters any tool proposal (tool
  *firing* lands in PR-2d-2). New read-only `ListReactTurns` RPC (instance-scoped,
  paginated) mirrors `ListReplanRounds`. Checkpoint format **v3→v4** (carries
  `react_rounds`; a v3 sidecar is refused and recovery full-folds, self-healing).
  Journal v7→v8 is a pure pass-through migration; the canonical demo digest
  `7d22d4bd…` is byte-invariant; the dep walls now also forbid `kx-model-harness`
  and `kx-mcp` below the gateway line.

- **GPU/Metal + decoding tuning for the in-process backend** (`crates/kx-llamacpp`).
  Env-driven knobs applied inside `ModelParams::new` / `ContextParams::new` — the
  exact constructors the runtime's dispatch path already calls — so they take effect
  with **no edit to the frozen trio**: `KX_N_GPU_LAYERS` (now **all layers offload to
  Metal by default on Apple**, CPU elsewhere — CUDA stays cloud-only, D28),
  `KX_FLASH_ATTN` (`auto`/`on`/`off`), `KX_KV_TYPE` (`f16`/`q8_0`), `KX_N_THREADS`.
  New `ContextParams::with_flash_attn`/`with_type_k`/`with_type_v` builders +
  `FlashAttn`/`KvCacheType`. Unset env = llama.cpp defaults (byte-identical; the
  determinism smoke + canonical digest are preserved). `just metal-smoke` witnesses
  real offload.
- **Qwen3 agent-model integration** (`crates/kx-model-harness`, `crates/kx-model-store`,
  `crates/kx-planner`). The model name is now configurable (`KX_MODEL_NAME`; default
  unchanged for identity stability); a fail-soft GGUF metadata reader
  (`kx_model_store::read_context_length`) lets the runtime size `n_ctx` to the model;
  a `register_kortecx` helper builds the model's `ModelDescriptor` +
  `ProvidedCapabilities` and asserts the validator returns `TypeOk` (Apache-2.0, Text,
  native tool-calling). The strict tool-call (`kx-model-harness`) and plan
  (`kx-planner`) decoders now tolerate a leading Qwen3 `<think>…</think>` reasoning
  block (leading-block-only — the fail-closed strict parse and SN-8 exact-grant
  matching are unchanged). `just fetch-agent-model` fetches a public Qwen3 stand-in.
- **Live model dispatch in `kx serve` (AL1, opt-in)** (`crates/kx-gateway`,
  `crates/kx-cli`). Built `--features inference`, the embedded worker runs **real
  model Motes** through the in-process llama.cpp backend: the new `kx/recipes/chat`
  recipe ChatML-wraps a `prompt` free-param, greedy-decodes, and commits the
  completion exactly-once. Composes the existing public `InferenceBackend` surface —
  **the frozen trio is untouched** — behind a `MoteExecutor` the gateway owns, and is
  **off by default** so the default `kx` stays FFI-free (the `build-no-inference` gate
  + the dep-wall stay green).
- **`frozen-trio` CI guard** (`.github/workflows/ci.yml`). A PR whose diff touches
  `kx-inference`/`kx-executor`/`kx-scheduler` `src/` fails the gate — the thesis test
  (layers-on-top must not edit the kernel) is now enforced, not just promised.

- **Real, sandboxed Mote body-execution in `kx serve`** (`crates/kx-gateway`).
  The embedded worker now runs a real Mote body inside the platform sandbox
  (bubblewrap on Linux, sandbox-exec on macOS) for the new `kx/recipes/exec-demo`
  recipe — materializing the body from its `logic_ref`, running it under the
  warrant's scope, and reconciling its output into the content store so the run
  commits exactly-once. The demo `echo` path and the canonical projection digest
  are unchanged (the frozen trio `kx-executor`/`kx-scheduler`/`kx-inference` is
  untouched — the gateway composes their existing public API). **Fail-closed:** a
  sandbox that cannot run errors rather than executing on the host. The runtime
  image ships `bubblewrap` + the demo body; real-exec under the hardened
  `docker-compose` is a documented `seccomp=unconfined` opt-in (Docker's default
  seccomp blocks the unprivileged user namespace bubblewrap needs).

## [0.1.0] — the reachable runtime

The first release where the durable runtime is **reachable end to end**: a server,
a CLI, recipes, an audit trail, and a live event stream, on top of the
exactly-once durability spine.

### Added

- **`kx` CLI** — one FFI-free binary (`crates/kx-cli`). `run`/`replay`/`digest`
  drive the engine locally; `serve` hosts the gateway; `invoke`/`submit`/
  `projection`/`content`/`events`/`signatures` are gRPC clients of a running
  gateway. Agent-ergonomic `--wait` runs the runtime like a function and returns
  one committed result; `--json` everywhere; a typed exit-code contract
  (`0` ok / `2` usage / `3` wait-timeout-resumable / `1` rpc+io).
- **Gateway server** — `kx serve` hosts the `KxGateway` gRPC service over an
  embedded coordinator + local worker (`crates/kx-gateway`, `crates/kx-gateway-core`).
  Bearer-token auth with **deny-all default** and **server-derived identity**;
  `--dev-allow-local` for loopback development.
- **Inbound recipe execution** — `Invoke` binds a published recipe by handle to
  JSON args and runs it to a committed terminal Mote, exactly-once
  (`crates/kx-invoke`).
- **Recipe library + prompt templating** — five reusable, deterministic recipes
  (`map_reduce`, `fan_out_gather`, `retry_until_critic`, `react_tool_loop`,
  `image_batch_describe_reduce`) and a pure, fail-closed prompt-template engine
  (`crates/kx-workflow`).
- **Audit trail** — an off-truth-path, best-effort JSONL audit sink that records
  the run lifecycle without ever touching the projection digest
  (`crates/kx-audit`); enabled with `kx run --audit-log <path>`.
- **Live event stream** — `StreamEvents` is a true resumable live tail, with a
  WebSocket bridge; `kx events --follow` consumes it and auto-resumes.
- **Durable catalog & fleets** — a sharable signature/recipe catalog with durable
  SQLite-backed ledgers (`crates/kx-catalog`) and team/fleet membership
  (`crates/kx-fleet`).
- **Tiered install automation** — `just setup` (FFI-free), `just setup-inference`
  (opt-in native backend), `just fetch-demo-model` (SHA-256-verified GGUF), a
  tiered `just doctor` with per-OS install hints, and `just verify-quickstart`
  (a docs-as-test gate that runs the README quickstart and asserts the canonical
  digest).
- **Documentation** — a production-grade README (quick start → serve → inspect),
  refreshed `GLOSSARY.md`, and this changelog.

### Guarantees (carried from the durability spine)

- A world-mutating step takes effect **exactly once** across crashes, retries, and
  redistribution.
- All live state is a **pure fold** of an append-only journal; recovery re-folds
  the log. Cold re-fold of a 25k-Mote journal stays sub-linear (gated in CI).
- The `kx` binary installs with **Rust only** — no C++ toolchain (proven by a
  dependency-wall test and an FFI-free CI build job). llama.cpp is opt-in for local
  inference.

### Known limitations

Plaintext gRPC (front with TLS for non-loopback); bearer-token auth with no
multi-tenant isolation yet; single-system journal writer; single-stream inference
with model-by-path (no registry); audit-log + event-stream observability (no
metrics/OTel export yet). See the README's *Production notes & known limitations*.

[Unreleased]: https://github.com/Kortecx/kortecx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Kortecx/kortecx/releases/tag/v0.1.0
