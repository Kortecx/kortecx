# Changelog

All notable changes to kortecx are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). kortecx is in early
development; interfaces may change before 1.0 — pin a commit if you build on it.

## [Unreleased]

### Added

- **Apps — the durable, shareable unit of agentic capability.** An App is a
  `kortecx.app/v1` envelope that wraps a portable blueprint with by-reference
  context / tool / connection / dataset references, a prompt/rule/skill/memory
  rail, and a steering config (model, max turns, max tool calls).
  `kx app new/save/list/get/run/export/import/clone` — plus the `kx.app(...)` /
  `app(...)` builders in the Python and TypeScript SDKs and the console **Apps**
  section — author, run, and share them. An App carries **no authority**: `run`
  and `import` re-resolve every warrant from the caller's own grants, and a shared
  bundle re-registers connections/secrets by name so it resolves each operator's
  own credentials. Off-journal + additive (the canonical projection digest is
  unchanged). (cli/sdk/ui)

- **Chains — a string-DSL for composing published task handles into a DAG.**
  `kx chain run "<dsl>"` (and `chain(...)` in both SDKs) lowers an expression built
  from `>` (sequential — a data edge), `&` / `|` (parallel merge), and `[ … ]`
  (grouping) through the **same** compile + warrant path as a blueprint — a chain
  only changes how the topology is authored. `--emit-blueprint` writes a portable
  blueprint; `--dry-run` lowers + validates offline. (cli/sdk)

- **Swarms — multi-agent patterns without hand-writing the DSL.** `kx swarm` (and
  `swarm()` / `supervisor()` / `consensus()` / `team()` in both SDKs) compose N
  agents into a fan-out → gather, a lead-plans / team-executes / lead-integrates
  topology, or a best-of-N vote (an LLM judge or an exact-equality majority). Pure
  client composition — the server compiles + warrants every step. (cli/sdk)

- **Skills — declarative capability bundles (`kortecx.skill/v1`).** A skill is
  instructions plus a tool grant-*wish* set; `kx skills add/list/show/remove` and
  `kx new skill` manage the per-principal catalog (mirrored in both SDKs). Adding a
  skill grants nothing — at run the server intersects the wish against your grants
  and the live broker (wish ∩ grants ∩ fireable), and attaches it to an App via
  `kx app new --skill` or the SDK `.skill(...)` builders. (cli/sdk)

- **Durable agentic memory (`kx memory`).** `add / list / recall / forget / decay /
  stats / restore / consolidate` (mirrored in both SDKs) let an agent remember
  facts and recall them across runs. Server-embedded and scoped to the caller's
  principal; recall scores are display-only and never an identity input. Enabled on
  an inference build with `KX_SERVE_MEMORY=1`. (cli/sdk)

- **Observability, cost & metrics surfaces.** Per-mote execution telemetry and a
  per-model token rollup (`kx telemetry`), a per-run local spend estimate
  (`kx cost`), a terminal-failure alerts inbox (`kx alerts`), 👍/👎 feedback
  (`kx feedback`), and an **opt-in Prometheus `/metrics`** endpoint
  (`--metrics-listen`, RED metrics, FFI-free). All audit/display-only — never
  truth, identity, or a projection-digest input; input-token counts are not
  measured in the OSS backend. (serve/cli/sdk/ui)

- **Bundled Slack and Notion MCP connectors.** `kx-connector-slack`
  (`post_message` / `read_channel` / `search` / `list_channels`) and
  `kx-connector-notion` (`search` / `read_page` / `create_page` / `append_block`)
  join Gmail and Discord under `integrations/`. Each is a standalone stdio MCP
  server the runtime dials via `kx connections add` (curated one-click
  `--provider slack` / `--provider notion`), authenticates by-reference (the secret
  is injected by name and never leaves the connector's own process), and ships an
  offline `*_FAKE` mode for tests. No `kx-*` runtime dependency, so building or
  running one cannot move the projection digest. (integrations)

- **Self-contained portable RAG — a shared App carries its own corpus.** A dataset
  reference may now carry the content it spans (`references.datasets[].cas_refs`, shipped
  by `kx app export --bundle --with-data`), and an importing server **materializes that
  corpus on first run** — so a shared App grounds on the bytes it travelled with, none of
  the author's datasets required and nothing to pre-ingest. The physical index is scoped
  (`<declared>.app-<hash>`, keyed on the corpus and the live embed model) so a carried
  corpus never merges into a same-named local dataset of yours, and an embed-model swap
  re-derives it rather than querying an incompatible index. Corpora are embedded
  server-side, so they must be UTF-8 text. An App exported *without* `--with-data` still
  falls back to grounding on a pre-ingested dataset of the declared name, exactly as
  before.
- **Integrations foundation: a local secret store, an event-trigger seam, and an
  Integrations hub.** Three additions let local agents authenticate real services and
  be driven by inbound events — the foundation for app/connector integrations.
  - **Local secret store (`kx secrets`).** A connector credential can now be stored in
    the OS keychain (macOS Keychain / Windows Credential Manager / Linux kernel
    keyutils) instead of only a host environment variable. `kx secrets set/list/rm`
    (and `kx.secrets.*` / `client.secrets.*` / the console Secrets panel) manage them;
    a connection's `credential_ref` resolves from the keychain first, then the
    environment (existing env-var credentials keep working). Secrets are referenced
    **by name** — the value is read transiently when the connector dials and is never
    journaled, never in a run's identity, never in the model's context, and never on
    any list/response (only names + timestamps surface). Secret writes require a
    loopback-bound gateway. The hardened multi-tenant KMS/HSM vault remains a Cloud
    capability.
  - **Event triggers (`kx triggers`).** A trigger binds an inbound source — a webhook,
    a local cron interval, or a direct `SubmitTrigger` call — to a recipe handle; when
    the event fires, the runtime starts a fresh durable run through the existing Invoke
    path (the trigger is the run's origin; a replayed event with the same idempotency
    key fires nothing and returns the prior run). A new opt-in **`--webhook-listen
    <addr:port>`** serves the untrusted-inbound surface with per-trigger HMAC-SHA256
    (`X-Kx-Signature-256` over the raw body, constant-time) or bearer auth, a payload
    cap, a per-trigger rate limit, and idempotency dedup; `none`-auth is permitted only
    on a loopback bind. `kx triggers add/list/test/fire/rm` (and `kx.triggers.*` /
    `client.triggers.*` / the console Triggers panel) manage them. The hosted
    multi-tenant trigger gateway at scale remains a Cloud capability.
  - **Integrations hub** in the console (the Tools section) surfaces Connections +
    Triggers + Secrets together. Docs: *Managing secrets* and *Setting up a trigger*.
  - Built with **no edit to the frozen trio**, the canonical projection digest
    unchanged (all new state is off-journal: the OS keychain + off-digest
    `triggers.db` / `secret_index.db` sidecars), and additive-only proto.

### Fixed

- **Stale per-model recipe after a model/engine switch.** Reusing a `--catalog-dir`
  across a model or engine switch no longer leaves a per-model chat recipe
  (`kx/recipes/m-<id>`) bound to a model the server no longer serves (which previously
  made every run of it fail closed). On startup the server now retires the grant for
  any such recipe, so it disappears from `Invoke` / `ListRecipes` / `ListModels` and
  only currently-served models are offered.

- **Live tool-calling for runtime-dialed MCP connectors.** An external connector
  registered at runtime (`kx connections add` / `flow().with_mcp(...)` /
  `RegisterMcpServer`) is now reliably callable by the autonomous loop: the tool-call
  parser also accepts a bare paren call (`server/tool(arg="…")`) some local models
  emit, and a model that names a tool ambiguously (a bare leaf shared by two connected
  servers, e.g. two `echo` tools) gets a precise, disambiguating re-prompt naming the
  full `server/tool` ids instead of the chain silently stalling. A dead-lettered
  agentic turn now always reports a reason (the last refusal, a spent budget, or a
  dispatch failure) instead of a blank terminal. **`kx connections fire --name <server>
  --tool <remote> --args '<json>'`** (and `kx.connections.fire(...)` / `connections.fire(...)`
  in the Python/TypeScript SDKs, plus a per-connector **Fire a tool** panel in the
  console) exercises one registered tool live through the broker — a model-free "does
  this connector work" check (it validates args against the tool's schema and enforces
  the same grant gate; it is a diagnostic, not a recorded run). (serve/tools/SDK/CLI/UI/docs)

- **Gemma-4-12B omni support + model-agnostic prompt templating.** A model-serving
  gateway now formats every model with its OWN chat template — applying the GGUF's
  embedded template through llama.cpp where it renders, with a built-in
  per-architecture fallback (`ChatML` / Gemma) for models llama.cpp cannot render
  (such as Gemma-4) — so a model is never fed another model's format. A recipe's
  structured reply is normalized symmetrically: a leading reasoning block
  (`<think>` or Gemma's reasoning channel) or a Markdown JSON code fence around a
  plan / tool-call envelope is stripped before the fail-closed parse. Pull and serve
  the recommended local model (Apache-2.0, text + image) with `just
  fetch-gemma-model` and `just review-serve-gemma`. (serve/inference/docs)
- **Data Lab — a multi-modal asset viewer + the datasets keystone.** Committed run
  artifacts and retrieval hits now render **inline in the browser** by kind: images,
  video, and audio (from a `blob:` object URL — never a remote `src`, so no
  outbound-fetch surface), markdown (React-element rendering, never `innerHTML`), JSON
  and text (read-only Monaco), with a bounded hex preview + byte-accurate download for
  anything else. The Datasets section is reframed as the **Data Lab** with a top-k
  slider, a `content_ref` chip, and a click-to-expand hit detail that renders through
  the shared viewer. A new **`kx datasets` CLI** (`list` / `ingest` / `query`, with
  `--json`) exposes the RAG data-plane, mirrored by the Python and TypeScript SDKs.
  (serve/cli/sdk/ui/docs)
- **`FuzzyDiscovery` — advisory fuzzy-in / exact-out retrieval (Slice-B).** A new
  additive RPC over a dataset's vector index that returns only content-addressed refs
  + a display-only basis-point score (SN-8 — never an identity input); resolve bytes by
  the exact ref. Exposed in the Python/TypeScript SDKs and an advisory "Discover" mode
  in the Data Lab. (serve/sdk/ui)

### Changed

- **The bootstrap demo team is now a workspace team** (`kx/teams/workspace`) whose
  members are the real configured parties (the `--auth-token` parties + the
  `local-dev` dev principal) — no fabricated/demo identity. **Upgrade note:** on a
  REUSED `kx serve` data dir the old `kx/teams/demo` rows are orphaned (the
  membership/grant ledgers are append-only and never delete), so both the old demo
  team and the new workspace team appear until the data dir is reset — a **fresh data
  dir is recommended** on upgrade. (gateway/UI)
- **`kx/recipes/fanout-demo` is renamed `kx/recipes/passthrough-dag`** — an honest
  multi-node fan-out → gather DAG whose every node passes its real input through.
- **`kx-content` / `kx-projection` / `kx-coordinator` / `kx-worker` bumped to `0.2.0`.**
  The `SharedContent` object-safe content seam (`trait SharedContent` + `type
  SharedStore = Arc<dyn SharedContent>`) retypes **`kx-coordinator` and `kx-worker`**
  public signatures from the concrete content-store type to `SharedStore` — a breaking
  change under Cargo's 0.x SemVer rules (a public-signature change ⇒ `0.1.x → 0.2.0`,
  not the `0.1.2` patch first published). `kx-content` and `kx-projection` are additive
  (a new trait/type and a new verdict variant) but move in lockstep to keep the seam on
  a single version line. In-tree callers are unaffected — `Arc<LocalFsContentStore>`
  unsize-coerces to `Arc<dyn SharedContent>` — but an external `^0.1` consumer that
  named the old concrete store type must update.

### Removed

- **Demo scaffolding (Golden Rule 15 — real-model integrity).** The `kx submit --demo`
  CLI verb, the `kx/recipes/exec-demo` recipe (and its `KX_DEMO_BODY_PATH` override),
  and the fabricated `"kx demo result for mote …"` placeholder are gone. Every runnable
  surface now produces **real** output — an honest deterministic passthrough for PURE
  steps, or real on-device model inference for model recipes. Use `kx invoke
  kx/recipes/echo` (or any published blueprint) instead of `kx submit --demo`. The
  platform sandbox machinery is retained as a stable seam for a future tools/scripts
  capability.

## [0.1.1] — 2026-06-10

A patch release from the clean-install verification campaign — two bugs caught
by testing the **installed** runtime end-to-end across all four surfaces (CLI,
Python SDK, TypeScript SDK, UI).

### Fixed

- **Morphic Data Engine: capture records are now correctly stamped with the run
  instance** (was all-zeros in a real `kx serve`). The serve-path capture poller
  folds the journal in ~250 ms ticks; the run instance is now persisted durably
  (`capture.db` `run_meta`, schema v1→v2) so an action committed in any later tick
  is stamped, not only one folded in the same tick as `RunRegistered`. `capture.db`
  is a rebuildable cache, so an old sidecar drops-and-rebuilds on first open.
  (gateway; OSS #172)
- **SDKs: `invoke(wait=True)` on `kx/recipes/react` no longer spuriously times
  out.** A ReAct chain has no statically-known terminal Mote (the run-salted
  turn-0 id is server-derived), so both SDKs now wait on chain **settlement via
  `ListReactTurns`** (answer → committed, dead-lettered → failed). Drive a react
  run's completion from a client/UI via `ListReactTurns`/events. (Python +
  TypeScript SDKs; OSS #173)

## [0.1.0] — 2026-06-10

The first public release: a single-system durable agentic-execution runtime
(`kx run` / `kx serve`) with the live agentic loop (plan, re-plan, critic,
ReAct-with-tools), the Morphic Data Engine (durable serve-path capture), the
Datasets/RAG data-plane, teams/grants viewers, a React+Vite console, and
Python + TypeScript client SDKs. Install the FFI-free `kx` binary via
`curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh`
(SHA-256-verified prebuilt for linux-x86_64 / linux-aarch64 / macos-arm64), or
`cargo install --path crates/kx-cli` from source. The canonical demo digest is
`7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9` (exactly-once
across a clean run and a crash-then-replay). Highlights since the pre-release
work — the entries below were developed under `[Unreleased]` and ship in 0.1.0:

### Added

- **Morphic Data Engine — durable serve-path capture** (`crates/kx-gateway`,
  `crates/kx-gateway-core`, `crates/kx-proto`, SDKs). On-by-default step capture
  (`kx-capture`) previously ran ONLY in the single-node `kx run` engine and held
  its records in memory; `kx serve` captured nothing. It now logs in serve: a
  background poll-fold of the gateway's **read-only** journal handle into a
  durable `capture.db` sidecar under `--catalog-dir` — off the sole-writer commit
  path (zero added commit latency; the canonical digest `7d22d4bd…` is
  byte-invariant, I1.c-proven) and off the truth path (a **rebuildable cache**:
  on a stale schema, a torn DB, or a deleted sidecar it drops-and-rebuilds from
  the journal, which stays truth — D40). Records are **join-key-only** by
  construction (the schema has no payload/reasoning columns — the privacy-safe
  ActionsOnly scope made structural; `Full` stays code-gated): a committed Mote's
  `mote_id` / `instance_id` / `result_ref` / `nd_class` / `seq`, plus the ReAct
  `turn`/`branch` joined from the chain's off-DAG `ReactRound` facts. Queryable
  via the additive read-only **`ListCaptureRecords`** RPC (instance-scoped,
  paginated, newest-first) and the new `list_capture_records` wrappers in both
  SDKs. The capture ledger lives in the `kx-gateway` host (the dep walls forbid
  `kx-capture` in `kx-gateway-core`); gateway-core gets only a capture-free
  `CaptureView` seam. `rusqlite` (already in the default closure via
  kx-catalog/kx-fleet; pure-Rust C, not the llama.cpp FFI) is now a direct
  non-optional dependency. FFI-free build unaffected.

- **SDK ReAct / re-plan / capture queryability + v0.1.0** (`bindings/python`,
  `bindings/typescript`, all crates). `ListReactTurns` / `ListReplanRounds` /
  `ListCaptureRecords` gained high-level client wrappers (Python sync + async;
  TypeScript) with frozen page types and from-proto tests — the UI extension can
  now surface a chain's Reason→Act→Observe history, a run's re-plan rounds, and
  the action exhaust. All crates bumped `0.0.1` → `0.1.0` for the first public
  release; a new `just features-guard` keeps the installed-binary feature matrix
  (`--features hnsw`, `--features inference,hnsw`) buildable + FFI-free.

- **Live ReAct TOOL FIRING in `kx serve` (PR-2d-2, react-tools-live)**
  (`crates/kx-mcp`, `crates/kx-coordinator`, `crates/kx-worker`,
  `crates/kx-gateway`, `crates/kx-gateway-core`, `crates/kx-projection`,
  `crates/kx-proto`, `crates/kx-profile`). The PR-2d-1 answer-only fence is
  replaced by the live tool round: a committed turn that proposes a
  warrant-granted tool now has its decision **validated at the freeze** (the
  sole-writer settle resolves the tool against the registry and checks the args
  against its typed `inputSchema`, fail-closed — a frozen `Tool` fact is always
  fireable), then the coordinator **materializes the OBSERVATION Mote**
  (byte-identical to the harness `react_tool_mote_salted`, cross-impl golden
  pinned on both sides of the dep wall) whose commit gates the next turn — the
  harness fire-then-bound order, crash-flavor guard included (a reaped worker's
  late observation commit still advances the chain). Args travel **out-of-band**
  of the Mote identity: an additive `WorkItem.tool_args` carries the
  coordinator-validated `(args_bytes, net_scope)`, **re-derived at every
  (re-)lease as a pure function of committed facts** (nothing staged, crash-safe
  by construction); the worker consumes it into the `EffectRequest` and
  **refuses to fire a granted tool without args** (terminal, F4). The first
  `kx-gateway→kx-mcp` edge lands as an OPTIONAL dep behind `inference` (the
  dep wall moves it from FORBIDDEN to the hnsw-style optional-edge proof), with
  a new bundled deterministic stdio tool (`[[bin]] kx-mcp-echo`, `mcp-echo@1`,
  no egress) registered on the serve broker, and **`kx/recipes/react`**
  (free-params `instruction`/`max_turns`/`max_tool_calls`, validated
  `0 < max_tool_calls < max_turns ≤ 8`; the durable anchor records the bound
  caps) provisioned under the SERVER-constructed tool-granting react warrant —
  the first non-empty `tool_grants` in serve. Admission hardening: `SubmitRun`
  now **refuses any client warrant carrying `tool_grants`** (tool authority is
  server-issued only — the red-team BLOCKER #5 / Morphic finding), refuses
  `react_seed` on a serve without the inference executor (the
  `critics_supported` twin), and `Invoke` refuses a recipe granting a tool the
  broker never registered. The F-7 react trajectory now interleaves
  observations in transcript order (`[turn0, obs0, turn1, …]`). `kx-projection`
  gains a DERIVED per-instance `react_rounds` index (+ a react-turn-Mote set)
  — settle/recover/trajectory reads are now per-chain, closing the PR-2d-1
  O(runs²) finding at the source; the index is never serialized (checkpoint
  stays **v4**; `encode_state` and the canonical demo digest `7d22d4bd…` are
  byte-invariant; **no journal schema bump** — observations commit as ordinary
  entries). `kx-profile` gains M7a (react answer-settle) + M7b (full tool round
  firing the real bundled tool) spikes. The worker gains the **react-turn
  routing arm** the substrate was missing in a real serve: a coordinator-
  materialized TURN (ROND, the identity-bearing marker, no `tool_contract`)
  dispatches directly through the hosted executor (whose react arm decodes +
  fences pre-commit) — previously every non-PURE Mote routed to the capability
  broker, so a live react turn could never reach the model (caught by the new
  `react_serve` e2e, the first to drive the chain through the real serve stack:
  Invoke → real Qwen3 inference per turn → settle → `Answer` via
  `ListReactTurns`).

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
