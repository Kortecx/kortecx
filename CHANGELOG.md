# Changelog

All notable changes to kortecx are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). kortecx is in early
development; interfaces may change before 1.0 — pin a commit if you build on it.

### Added

- **A connector can now be given the whole environment it needs, by reference.** A stdio MCP
  server used to receive exactly one environment variable, and its name was forced to be the
  name of the stored secret — so any server wanting two was impossible to configure. A
  connection now carries an environment map: each entry pairs the variable the server reads
  with the NAME of a secret that supplies it. Both halves are names; the value is resolved when
  the server starts and dropped, so nothing is written to disk. Available as repeatable
  `kx connections add --env NAME=SECRET_REF`, as `env` on `registerMcpServer` (TypeScript) and
  `register_mcp_server` (Python), and as environment rows in the console. `ListMcpServers` and
  `kx connections list` report the variable NAMES a connector is configured with — never the
  secrets behind them. For example, the official GitLab MCP server needs both
  `GITLAB_PERSONAL_ACCESS_TOKEN` and `GITLAB_API_URL` and could not previously be dialed at all:

  ```sh
  kx secrets set --name gitlab-token --value '…'
  kx secrets set --name gitlab-url   --value 'https://gitlab.example.com/api/v4'
  kx connections add --name gitlab --command mcp-server-gitlab \
    --env GITLAB_PERSONAL_ACCESS_TOKEN=gitlab-token \
    --env GITLAB_API_URL=gitlab-url
  ```

  A malformed map is refused at registration with the reason (an empty variable name, a
  variable declared twice, or an entry naming no secret), and a variable whose secret does not
  resolve refuses the connection rather than starting the server without it. `--env` takes the
  name of a stored secret and rejects anything that looks like a value.

  An environment map is set by an operator and is **never model-proposed**. It travels on a new
  `RegisterMcpServerWithEnv` RPC rather than on `RegisterMcpServer`, because the plain
  registration request is also what a natural-language proposal displays, logs and forwards —
  and an environment decides what a child process is configured with. Keeping the map off that
  message makes "a model cannot propose an environment" a property of the schema rather than a
  rule a proposer must remember, the same reason `ProposedScript` omits `argv`. The CLI and both
  SDKs pick the right call for you, so registering with an environment is still one command.

### Changed

- **A step on a run's graph now says which turn it was and which tool it ran.** A step was
  labelled with an internal identifier and a determinism class — `f3da775e…4f64 /
  READ_ONLY_NONDET` — while the Timeline tab described the same step of the same run as
  `Turn 0 · MCP · mcp-echo/echo@1`. The graph now shows the turn, the kind of step and the tool
  and version it called, in the same words the Timeline uses, from the same records; the full
  identifier is still one click away in the step's detail panel.

- **Tools → Integrations and Tools → Connections are now one Connectors surface.** They
  described the same registry from opposite ends: one listed the connectors that ship in the
  box and could only print a command to copy into a terminal, the other listed what was
  actually dialed and knew nothing about what shipped. There is now a single list — every
  bundled connector plus anything else you have connected, each row saying whether it is set
  up, and setting one up is an action on the row. Links to the two old tabs still open the new
  one.
- **`kx react list --json` now includes `step_salt`.** The chain key was on the wire and in the
  text output but missing from the JSON, so a caller could not scope a query to a single
  agentic submission without it.
- **The release now ships the runtime's own agent tools.** The prebuilt release carries a
  `kx-tools-<target>.tar.gz` beside the `kx` binary: the bundled deterministic stdio tools
  (`kx-mcp-echo`, `kx-mcp-calc`, `kx-mcp-kv`) and the four bundled connectors
  (`kx-connector-{gmail,discord,slack,notion}`). The installer verifies and unpacks them beside
  `kx`, where the runtime resolves them — so on a fresh `curl | sh` install, `kx chat --tools`
  and `kx agent run` now actually run an agent, `kx connections add --provider gmail` dials a
  binary that exists, and `kx connections doctor` reports all four connectors resolvable.
  Previously the release shipped only `kx`: the agent recipe was never seeded and the agent verbs
  failed with an unexplained authorization error. Installing an older release still works — the
  installer says plainly when a release predates the tools bundle (`KX_SKIP_TOOLS=1` skips it
  explicitly).
- **Bundled tool binaries resolve beside the executable.** The runtime now looks for its bundled
  tools next to the running `kx` binary (after the explicit env override and the container image
  path) — which also makes the from-source path work: `cargo install --path crates/kx-mcp --bin
  kx-mcp-echo --bin kx-mcp-calc --bin kx-mcp-kv` puts the tools beside a
  `cargo install`ed `kx`. A missing bundled tool is now a loud serve-boot warning instead of
  silence, and `kx agent run` explains exactly what is missing (model or tool binary) instead of
  failing with a bare permission error.
- **The release packaging is proven before a tag exists.** A new `verify-release-parity` gate
  builds the release artefacts with the same script the release workflow runs, installs them with
  the real installer, and asserts the installed binary reproduces the canonical digest, registers
  its bundled capabilities, seeds the agent recipe, and accepts the README's headline command —
  hermetically, on every change. The installer itself gains a base-URL override for exactly this
  kind of pre-release verification; its production default is unchanged and asserted.
- **Doc examples now execute as written.** `kx connections add --command` takes one program path
  (spawned directly, no shell) with each argument as its own `--arg` — the examples that packed a
  whole `npx …` command line into one string could never spawn and are fixed everywhere they
  appeared, as is an example using a flag the agent verb does not have. The `KX_SERVE_FS_ROOT`
  variable the README's headline chat depends on is now documented beside that command, and
  `--features …` is no longer presented as a `kx serve` argument (it is a cargo build flag).
- **A served model now tells the runtime how it spells a tool call.** The argument grammar was
  armed on one syntax — the JSON envelope the runtime's own prompt teaches — so a model that
  proposes tool calls in a different one had its arguments checked by nothing: the call was
  recovered by the tolerant parser, refused by the argument schema, retried, and the run failed
  having looked like it worked. A GGUF's chat template renders the delimiters, so the runtime now
  reads them back at load and constrains what the model writes. On the same model, `task_success`
  on llama.cpp moves from 826 to 934 — `http`, `long`, `reach`, `script` and `failure` all reach
  1000 — while every Ollama family is byte-identical in the same capture. One number moved the
  other way and is published rather than withheld: `memory_quality` on llama.cpp reads 0, and the
  README says why.
- **Adding a model is a command rather than a code change.** `just model-conformance <path.gguf>`
  derives the model's dialect, checks the sampler engages, checks ordinary prose is not masked,
  and checks the arguments it emits are legal for the tools it was granted. It fails rather than
  skips when no model is available, because a skip reads exactly like a pass.
- **The published benchmark block is generated from the committed baselines.** The chart, the
  denominators and the capture provenance are rendered rather than hand-maintained, and CI now
  checks the surrounding prose against the baselines too — not only the tables. The published
  comparison covers the agentic families; the authoring and scripting families are disclosed as
  an aggregate rather than dropped, and CI checks that the two reconcile.
- **Benchmark baselines are now captured on the same model family for both engines.** The two
  committed baselines were captured on different models, so no engine comparison drawn from them
  was valid. Both are now Gemma-4-family captures — `gemma4:12b` on Ollama and the same-family
  GGUF on llama.cpp — with the embedding model recorded in each capture's environment. The Ollama
  numbers are a fresh reading (the old baseline was a different model, so no delta is meaningful);
  the llama.cpp numbers are a valid like-for-like recapture. The README and evaluation tables
  moved with them.

### Fixed

- **Steps on a run's graph no longer draw on top of one another.** The graph reserved a fixed
  box per step that was less than half the height a step card actually occupies, so on a run of
  more than a couple of steps the rows were placed closer together than the cards are tall and
  the cards overlapped — on a four-turn agent run, half of all step pairs. The graph now measures
  a rendered card and lays out against that, so a card that grows a row moves the layout with it
  instead of re-opening the same problem.

- **The run graph is sized and oriented for the window it is in.** The canvas was a fixed height
  regardless of screen, and always stacked steps top-to-bottom — so a long agent run in a short,
  wide window was drawn tiny with most of the canvas empty. The canvas now scales with the
  viewport, re-fits when the window is resized, and lays a run out left-to-right when that
  genuinely fits better, keeping top-to-bottom otherwise so a small resize cannot spin the
  picture around.

- **The run view can now isolate an agentic run started with `Invoke`.** The identifier the
  server returned for such runs named an internal placeholder that never appears in the run's
  projection, so the console could only fall back to showing every step in the server's journal
  behind a warning — and `kx invoke --wait` on the built-in react recipe could never observe
  completion. The server now returns the chain's first admitted step as the run's anchor, the run
  view scopes to it (retrying with the terminal id when a saved link carries a stale anchor), and
  `--wait` / `--stream` on such runs observe real progress. The turn Timeline is unaffected.
- **The dual-engine benchmark recapture script can complete its Ollama arm again.** It left the
  llama.cpp model configured while turning Ollama on, which the bench's one-engine-per-run guard
  correctly refuses; the second arm now clears the first arm's engine configuration.

### Added

- **The live model test suites now run without a C++ toolchain.** Fifteen test files that drive a
  real served model were gated on the in-process llama.cpp backend, so they were compiled away
  entirely on a build that talks to Ollama instead — reporting zero tests rather than reporting
  that they had been skipped. Forty-seven live tests were affected, including the only coverage of
  authoring a workflow from a description and of holding a tool call at an approval barrier. They
  are now gated on the model-serving feature instead, so `cargo test --features serve-engine,hnsw`
  runs them against Ollama with no llama.cpp build. The llama.cpp arm is unchanged and still needs
  `--features inference`.

- **New end-to-end coverage for memory decay, grounded retrieval, skills and denied approvals.**
  A decay sweep is now proven to evict stale memories, spare a recently recalled one, and restore
  what it tombstoned. A retrieval answer is proven to actually contain what the search returned,
  rather than only that a search happened. A skill's instructions are proven to change what a live
  model answers. And a denied approval is proven to leave the withheld action unperformed.

- **Tool arguments are now constrained on the Ollama backend too, not just llama.cpp.** When a
  registered tool declares typed parameters with a closed schema, the runtime already told the
  llama.cpp backend exactly what shape the arguments had to take. Ollama was only told "the
  arguments are an object", so a model could propose an argument the tool's schema forbids —
  the call was still refused before anything ran, but the turn was spent finding that out. The
  runtime now sends the declared parameters to Ollama as well: enum values, integer and boolean
  types, which arguments are required, and whether unknown keys are allowed. A tool with no
  declared schema is sent exactly as before.

- **`kx info` and the Models view name the model that produces your embeddings**, and benchmark
  captures record it. With no embedding model configured the runtime falls back to the chat
  model, which produces much weaker results for retrieval — visible now, rather than only as a
  low score later.

### Fixed

- **The console no longer shows internal tracking identifiers.** A few tooltips and one line of
  body text on the Models page ended with codes like `(D114)` that mean nothing outside our own
  notes. They now say what the thing does.

- **Disabled Share controls explain themselves without a hover.** On App and workflow cards the
  greyed Share icon offered its reason only as a tooltip, which reads as the feature being
  missing rather than unavailable. The reason is now visible text beside the icon.

- **Long text no longer gets clipped on the Models page or in a chat's title.** The two Models
  side panels were pinned to a narrow column and cut their own text off; a chat named after a
  long first message was truncated mid-word.

- **The workflow step drawer gained a persona picker and search.** The persona row now shows
  which persona is applied, lets you remove it by picking it again, and can be filtered. Long
  tool, skill, integration and grounding lists can be filtered too.

- **Offloading a model no longer disrupts live work without telling you.** `OffloadModel`
  destroys the model to free RAM, and it used to do that unconditionally — stopping a
  running app's model was one click, with no warning and no record of what it broke. The
  server now checks what is holding the model first. If live work holds it, the offload is
  **refused** rather than performed: nothing is evicted, and the response names the apps
  that would have been disrupted. Pass `force` to proceed anyway, and the response still
  lists what it disrupted. In the console the Models page shows the reason beside the
  button with an "Offload anyway" override; from the CLI, `kx models offload <id>` prints
  the holders and exits non-zero, and `--force` overrides it. A gateway that cannot
  determine usage reports that it did not check, rather than reporting an empty list —
  "nothing was checked" and "nothing is using it" look identical otherwise.

- **Gemma-4 is now prompted in its own format.** Chat templates are selected by the model
  family the server reports, and Gemma-4 matched no entry, so it received a format it was
  never trained on. It answered anyway, which is why this was not obvious — but its
  internal channel markers leaked into replies as ordinary text. Model families now live in
  one table both model backends read, so a family cannot be templated one way in one place
  and another way elsewhere; Gemma-3 was affected by that split and is also fixed. Adding
  support for a new model family is now a single entry in that table, and each family's
  stop tokens are derived from its template rather than maintained separately. No action
  needed — serve a Gemma-4 model and replies come back clean.

- **A scaffolded web app is now written against the version it actually installs, and
  styled with a stylesheet it actually has.** The instructions the runtime gives a model
  when it scaffolds a hosted app previously named the framework and listed the packages it
  must not add. They now also state the framework's major version, show the stylesheet
  import, say positively where styling belongs, and state that a hosted app runs with
  loopback-only network access — so nothing is fetched from the internet at build or run
  time. Generated pages had been reaching for a web-font API the sandbox refuses and for
  utility CSS classes the project never installs; both produce an app that starts and
  passes every check while rendering wrong. The runtime additionally guarantees the
  stylesheet and the test the instructions ask for: if the plan omits either, it is added
  rather than silently skipped. No action needed — scaffold an app and the generated code
  follows the new rules.

- **A control the server cannot offer now says so on the page.** Where a capability is
  unavailable — starting a hosted app on a server built without hosted-app support, for
  instance — the greyed control now carries a short reason beside it instead of only in a
  tooltip. A greyed icon with no visible explanation reads as a missing feature rather
  than a switched-off one.

- **Status colour now means something.** Run states, tool risk classes and notices share
  one set of semantic colours (success, warning, danger, information) in both light and
  dark themes, instead of seven shades of grey. Colour is never the only signal — each
  state still carries its own icon and label — and every pairing meets the same contrast
  bar the console already held itself to.

- **A refused agent turn now shows the model output it was refused on.**
  `ReactTurnSummary` gains `raw` — the model output a turn was settled from, present
  when the turn was rejected and capped by the server. `kx react list` shows it: the
  full value under `--json`, a single-line excerpt in the text listing. Previously a
  trajectory recorded only *why* a proposal was refused, which is enough for a schema
  error and not enough when the refusal came from a judgement about the output itself.
  Nothing is stored for it — the value is read from the turn's already-committed
  output — so no journal or schema version changes. `kx-proto` 0.16.0 → 0.17.0.

- **A permanent tool failure is no longer retried as a temporary one.**
  A dispatch refused for a fixed reason — an undeclared capability, a warrant that does
  not permit the call, a rejected credential — used to be retried on the transient
  budget before dead-lettering. Those retries could not succeed; they added latency and
  buried the original cause. Such failures now fail immediately, with the reason intact.
  Failures that a retry can genuinely survive (rate limits, timeouts, unreachable hosts)
  are unchanged.

- **An unavailable embedding model is reported instead of silently swapped.**
  If `KX_SERVE_EMBED_MODEL` names a model the server does not serve, startup now logs an
  error naming the model and what is available, and embeddings stay unavailable until it
  is fixed. Previously the server quietly substituted the primary chat model, which
  cannot serve embeddings — so every retrieval failed later, for a reason that pointed
  at the wrong place. Leaving the variable unset is unchanged.

- **The chat model picker no longer offers a model that cannot chat.**
  On a server with an embedding model registered and no explicitly-chosen default, the
  console's "Auto" selection could land on the embedding model, and the first message
  would fail. It now prefers the model the server reports as serving.

- **Benchmark comparisons state when the baseline was captured.**
  Gate results are ratcheted against a committed per-engine baseline. The comparison now
  prints the commit and capture date alongside the result, so a number is never read as
  current when the baseline predates the code being measured. The README benchmark tables
  carry the same note.

- **A failing tool's own error now reaches the model — journal schema v17 → v18.**
  When a tool ran and failed, the runtime told the model only which BUCKET the
  failure fell into: a nine-variant enum whose catch-all renders as *"it failed to
  run. Do not call it again with the same arguments."* A JSON-RPC
  `-32004 no such vessel "x"` means change the argument, so that steer was not
  merely vague, it was the opposite of the fix. The tool named its own failure and
  the runtime discarded the name.

  `JournalEntry::Failed` gains a trailing, length-prefixed `detail` carrying the
  diagnostic the failing subsystem itself produced. What may be shown to a model is
  an ALLOWLIST, not a stringify: only a capability's own failure reason qualifies,
  and every runtime-side diagnostic (sandbox refusals, content-store paths) renders
  as it did before. With no detail, the rendered text is byte-identical to v17, so
  no existing chain identity moves.

  **Operator impact: none, and no action is required.** A v17 journal is read by a
  v18 binary unchanged — a v17 `Failed` body carries no detail and up-converts to an
  empty one, so the migration is a pure pass-through, exactly like the trailing-field
  additions at v9/v11/v12/v14/v15. The product identity digest is unchanged. As
  always, an OLDER binary refuses a NEWER journal loudly rather than mis-reading it,
  so roll forward before rolling back.

- **Natural-language authoring across every authoring domain, and a durable
  Policy/Role registry.** `ProposeControlAction` turns one sentence into the
  EXACT typed request the runtime would issue, and returns it without writing
  anything; a human approves, and the client calls the ordinary mutating RPC with
  the bytes it was shown. There is no second approval mechanism and no
  re-derivation step between what you saw and what runs.
  `DescribeControlSurface` projects the generated ControlSurface so a client can
  render what is authorable at all.

  Two preview arms are deliberately REDUCED rather than being the real request
  message: secrets ride `ProposedSecretName` (a value must never appear on a
  response type) and scripts ride `ProposedScript` (argv and env are fixed by an
  operator and are never model-controlled). Because the wire cannot express those
  fields, a forwarded request necessarily has them empty — a property of the
  schema rather than a rule the proposer has to keep applying, and one a
  descriptor walk over this crate's own `FileDescriptorSet` asserts on every
  build.

  `kx policy put | list | delete | assign` manages durable roles. **A role
  NARROWS tool authority and never grants it**: effective authority is the
  intersection of every present leg, so assigning a role can only take capability
  away. A party with no role assigned resolves exactly as it did before the
  registry existed. `kx-proto` 0.15.0 → 0.16.0 (wire-additive; the service trait
  gains six methods, so breaking-in-Rust and a minor bump).

- **Upgrade tests now run against state dirs released binaries actually wrote.**
  `crates/kx-runtime/tests/fixtures/state-dirs/` holds two frozen directories
  captured by the published `v0.1.1` (journal v8) and `v0.2.0-rc.1` (journal v16,
  with a real App, trigger, branch and context bundle) binaries — downloaded from
  their GitHub releases and sha256-verified against each release's own
  `checksums.txt` before being executed. The previous only old-version fixture
  was built by writing a current journal and downgrading it with raw SQL, which
  can only contain what today's writer knows how to produce.

### Changed

- **Secrets are stored in a file you control, rather than the OS keychain.** The
  runtime keeps local credentials in `secrets.json` under its catalog directory,
  created owner-only (`0600`). You can open it, see exactly which credentials the
  runtime holds, and add one by hand as `"NAME": "value"` — `kx secrets put`,
  `list` and `remove` work exactly as before. The file is refused at startup if it
  is readable by group or others (run `chmod 600` on it) or if it does not parse;
  in either case it is left untouched for you to fix rather than recreated, so no
  credential is lost to a bad file. Resolving a credential from a host environment
  variable is unchanged, and still works when no file is present.

  **Breaking:** credentials previously stored in the OS keychain are no longer
  read. Store them again with `kx secrets put`. Note the values are plaintext on
  disk protected by file permissions — this is a local-first store, not a vault.

- `tools.db` now opens through the sidecar upgrade policy as `UserAuthored`. It was
  the last store holding authored work outside that policy — and it holds more than
  tools: every registered SCRIPT lives there too, so two of the six authoring
  domains were sitting outside the protection the other stores got. Existing
  databases are unaffected: the old opener stamps its version in a `metadata`
  table, the policy looks in `meta`, finds nothing, and takes the fresh-file arm
  where every statement is `CREATE TABLE IF NOT EXISTS` — the rows are untouched
  and the file simply gains `meta.schema_version`.
- Sidecar stores classified `UserAuthored` now open with `synchronous = FULL`
  rather than `NORMAL`. `NORMAL` batches fsyncs, which can lose the last
  transactions to a power cut; that is the right trade for a cache you can rebuild
  and the wrong one for work a user authored and cannot. Applies to all seven
  authored stores, not just the new one.

## [Unreleased]

### Added

- **Workflows become a durable entity: save, list, run, schedule and restore
  stored definitions — with http/wait/conditional step kinds, parallel groups
  under per-step failure policy, and a benchmark family that measures the
  runtime itself.** `SaveWorkflow`/`ListWorkflows`/`GetWorkflow`/`RunWorkflow`/
  `DeleteWorkflow` store a `kortecx.workflow/v1` envelope as canonical bytes
  (wishes-never-grants: the envelope carries no authority; every warrant is
  built server-side at run from the caller's own grants), with definition
  history riding the append-only branch sidecar (`ListBranchVersions`/
  `RestoreBranch` — a restore re-syncs the stored envelope) and a caller-stated
  draft lifecycle. Step kinds: `http` is a bundled builtin dialing under the
  egress kernel (SSRF vetting, per-call host scope, a credential resolved BY
  NAME at dispatch, refuse-not-truncate caps; transport errors and 5xx fail the
  effect so retry can engage, 2xx–4xx commit honestly); `wait` is a DURABLE
  journal-backed timer (journal v17 `TimerArmed`; a serve killed mid-hold
  re-arms at the JOURNALED instant after restart and fires exactly once —
  proven live by a `kill -9` across an armed 60 s timer on the served Gemma
  build); `conditional` evaluates a typed predicate over its parent's committed
  bytes and the untaken arm commits a distinguished skip sentinel (its steps
  provably never run — the arm's endpoint is never dialed). Parallel groups
  join by first-non-skip or k-of-n quorum; per-step failure policy is
  fail-fast, `retry{max,backoff}` (attempts are FRESH identities with fresh
  idempotency tokens after a durable backoff — a same-identity redispatch can
  never satisfy an identity-keyed refusal), or `continue` (the failure commits
  as a canonical placeholder and the join releases). Same-route model steps are
  sequenced by an authoring-time control edge. Triggers gain
  `workflow_handle` with kind-aware validation at registration, and a repeated
  fire-failure now dead-letters the trigger with the reason on `TriggerView`
  (default 5, `KX_TRIGGER_DEADLETTER_MAX`) instead of retrying forever. The
  console adopts the Apps surface: `/workflows/create` (embedded builder — the
  save IS the authoring act), `/workflows/def/<handle>` with lineage, run,
  schedule, history and delete, and one generic history drawer for every
  entity. Both SDKs speak the workflow surface, and `runApp(wait: true)` now
  settles on THE run it started instead of the first commit on a shared
  journal (both SDKs; the CLI's non-agentic wait keys on the run's terminal
  anchor too). `bench-v1` grows the `workflow` family (41 tasks, fourteen
  families): seven stored deterministic DAGs whose oracle tokens exist only on
  the harness fixture — sequential carry, 3-way parallel join, a conditional
  pair scored as one property, a real 3 s durable wait, an identity-keyed retry
  recovery, and continue-past-failure — with machinery sentinels
  (`workflow_wait_elapsed@timers`, `workflow_retry_attempts@retries`) and a
  model-free drive gate that pins every task's exact step count.
  (kx-proto 0.15.0, kx-app 0.3.0, kx-journal v17, kx-projection, kx-coordinator,
  kx-gateway, kx-gateway-core, kx-eval, kx-cli, ui, both SDKs)

- **Apps: a create journey with a terminal result, drafts you can act on,
  point-in-time project restore, real hosted isolation, a per-project guidance
  file — and a benchmark family that scaffolds a REAL app live and runs it.**
  `/apps` is HOME and `/apps/create` is the whole create journey (compose →
  live scaffold → an honest terminal dialog; a failed scaffold leaves a DRAFT
  with resume/discard, durable across restarts via a `scaffold_state` mirror —
  a failed scaffold used to read as `Writing` forever after a restart, and the
  App page's "Project ready"/failure states were structurally unreachable).
  Branches record every non-dedup mutation in an append-only, bounded history
  (`ListBranchVersions`/`RestoreBranch` — restore APPENDS, survives delete, and
  the App page grows a History drawer). Hosted apps run their dev server under
  the platform sandbox where the platform can hold it (macOS `sandbox-exec`,
  deny-default, workdir-RW + loopback-only; `KX_HOSTED_SANDBOX=on` fail-closed
  refuses elsewhere), every hosted and MCP stdio child gets a CLEARED
  environment with a minimal allowlist, and stop kills the whole process
  GROUP — the vite/next grandchild included. `.kortecx/agents.md` (seeded at
  scaffold, user-editable) steers every scaffold write and rides every run's
  context rail first, under a `guidance:` label. `bench-v1` grows the
  `scaffold` family (34 tasks): a LIVE model-authored scaffold followed by a
  run whose canary answer is underivable unless the generated project reached
  the run, with a `scaffold_completed@attempts` sentinel and per-task
  duration/file Spikes. (kx-proto 0.14.0, kx-gateway, kx-gateway-core, kx-mcp,
  kx-eval, ui, both SDKs)

- **The benchmark grows its industry legend: two new families, order-sensitive and
  retrieval-side scorers, a reliability gate, and a cost/latency record.** `bench-v1`
  spans twelve families and 32 tasks: an `irrelevance` family (BFCL-style relevance
  pairs — abstain when no granted tool applies, beside healthy look-alikes that catch an
  always-refuse policy) and a `memory` family (LongMemEval-shaped, judge-free: a
  knowledge update whose superseded value stays live in the store, and an abstention
  when memory holds no answer). New scorers: `tool_seq_fsa`/`tool_seq_psa` (NESTFUL-style
  sequence accuracy — the order-sensitive columns beside the order-tolerant
  `tool_call_f1`, which now goes N/A on an empty gold multiset instead of folding
  abstention in), `context_recall` (the judge-free RAGAS `NonLLMContextRecall` shape),
  `pass_k4` (tau2-style pass^k, K=4 fresh-serve trials over three corpus-flagged
  flagship tasks; per-task values recorded, the mean gated, a trials sentinel making a
  skipped phase fail by name), and `retrieval_success_at_8` (Success@k, binary
  single-relevant qrels over the 61-document near-miss corpus). The failure family is
  documented as what it measures: a tool-fault recovery rate over an
  error/garbage/hang/healthy-control taxonomy. Captures now also record output-token
  economy (tokens-per-task, tokens-per-success; no input-token figure exists because
  the runtime records none) and model-free RPC latency probes (StoreMemory /
  RecallMemory / QueryDataset p50/p95), committed as never-gated Spikes in the
  per-engine baselines and held to the published tables by `check-docs`. (kx-eval,
  kx-gateway)

### Changed

- **Upgrading no longer costs you your work: a schema bump preserves what you authored,
  a downgrade refuses instead of wiping, and `kx migrate` finally exists.** Every SQLite
  sidecar under `--catalog-dir` used to end its open the same way — on a `schema_version`
  mismatch, `DROP TABLE` and start empty. For derived caches that is free; for the stores
  holding **apps, workflows, branches (and their restore history), triggers, skills and
  secret names** it meant a version bump silently deleted authored work on the next boot.
  It was not hypothetical: `apps.db` had its schema version frozen at 1 with a comment
  explaining that bumping it *"would drop saved apps"*, so the destructive open was being
  routed around rather than fixed — which froze the schema too. One policy now decides,
  in one place: a store holding authored work is **renamed aside** to
  `<name>.db.v<N>.bak` and its rows re-imported by column intersection, a **downgrade is
  REFUSED** (an older binary cannot know what a newer schema meant, and emptying the file
  to make the boot succeed is not an acceptable answer), and a derived cache still
  rebuilds empty exactly as before. A corrupt or foreign file still recreates empty in
  both cases — there is nothing readable to preserve.

  **`kx migrate --journal <path>`** brings a journal written by an older kortecx up to
  the current schema. `migrate_and_verify` had shipped since M2.x-E and was reachable
  only from a crate that already depended on the journal, so in practice an upgrade meant
  `kx serve` refusing to start with a bare schema-version mismatch and no remedy named —
  and the obvious workaround for a dead-end error is to delete the journal, which
  destroys the run. The rewrite is verified rather than trusted: both journals are folded
  and the migration is refused unless their committed-facts digests are byte-identical,
  so an upgrade can never quietly change what your runs produced. The original is
  preserved beside the migrated one; `--out` writes elsewhere and leaves the source
  untouched; `--dry-run` reports and writes nothing. The boot refusal now names the
  remedy, and names it only when it applies — a journal from a *newer* binary is told
  there is no downgrade rather than sent to a migration that cannot help it.

  Two guards keep this from decaying: a source-level assertion that every sidecar routes
  through the one policy and that the six authored-work stores are classified as such
  (reclassifying one is now a visible edit, not a silent default), and a CI check that a
  PR touching a `*_SCHEMA_VERSION` constant also touches a migration site and this file.

- **A recipe's identity now covers the authority it runs under, so changing a served
  model, a granted tool set or a decode budget no longer fails the serve boot of an
  existing install.** A recipe body is stored under what it compiles to
  (`ManifestId`), and a step warrant was not part of that identity — so a
  warrant-affecting change produced different body BYTES under an UNCHANGED id, which
  the body ledger refuses as an immutability conflict. That refusal sits on the
  serve's startup path, so an upgraded binary failed to boot against any state dir a
  previous binary had seeded. `kx_workflow::Manifest` now folds each step's
  `warrant_ref` into `ManifestId` (domain tag `…/v2`), and recipe seeding advances the
  asset handle with a version successor rather than leaving it pinned to the
  superseded body — without that half, the boot would have stopped failing and started
  silently binding the PREVIOUS recipe. The immutability rule itself is deliberately
  unchanged: it is what stops a replacement body widening a recipe's authority under
  an unchanged id.

  **Upgrade note.** Every recipe id moves once, by construction. No action is
  required and no data is lost: a body ledger written by an earlier binary still
  opens (its rows are verified against the identity scheme they were written under —
  `Manifest::id_v1` is retained read-only for exactly this), superseded bodies are
  retained so an in-flight run pinned to an old id still resolves, and the handle is
  advanced to the current recipe on the next boot. A body matching NEITHER scheme is
  still refused as tampered. The canonical projection digest is unaffected — a
  warrant change moves no `MoteId`, because two runs differing only in authority are
  the same computation. `kx-workflow` 0.2.0 → 0.3.0, `kx-catalog` 0.1.1 → 0.2.0.

- **The release build now says what it contains: local observability is the opt-in
  `observability` feature, and the prebuilt binary excludes it.** One cargo feature
  carries the Prometheus `/metrics` listener, the `alerts.db` inbox and the
  `telemetry.db` execution-exhaust sidecar (the kx-otel edge is optional behind it).
  Source builds opt in with `--features observability`; against a release binary,
  `kx telemetry` / `kx alerts` surface the server's honest `unimplemented` and an
  explicit `--metrics-listen` is refused with an error naming the feature. The wire
  is untouched — no proto change, and the RPCs keep their designed degrade. Two new
  test walls prove the property in both directions (the kx-otel edge absent from the
  release closure; the RPCs inert and no sidecar created on a release-shaped build).
  The journal-fold surfaces — the Activity drawer, run metrics, health, `kx cost`,
  the audit log — are in every build. (kx-gateway 0.2.0, kx-cli 0.2.0-rc.2)

- **The journal announces its own commits, and nothing inside the serve polls it any
  more.** `kx-journal` gains a change-notification seam (`WatchableJournal`,
  `JournalSubscription`); the two event streams and the capture, telemetry, alerts and
  metrics folds all subscribe instead of re-reading `current_seq()` every 250 ms. An idle
  serve now performs **zero** journal reads where it previously performed sixteen a
  second, and commit→frame delivery drops from a quarter-second-quantized wait to
  single-digit milliseconds. Delivery is unchanged and remains exactly-once: each
  follower keeps its own cursor and reads the contiguous range it is owed, so a
  notification decides when to read, never what was written. Watches are keyed by journal
  *file*, so the serve's writer handle and read handle share one. Client-side waiting
  (`--wait`, the SDK helpers) is a separate, over-the-wire seam and still polls.
  `KX_SERVE_JOURNAL_WATCH=off` restores the previous 250 ms cadence. (kx-journal,
  kx-gateway)

## [0.2.0-rc.1] — 2026-07-25

The first release candidate. Everything below has accumulated since 0.1.1: **Apps** — the
durable unit of agentic capability, authored from a sentence, runnable on a schedule or
served as a real web project, and callable by each other — plus chains and swarms, skills,
durable memory, the bundled MCP connectors, the datasets/RAG data-plane with
graph-augmented retrieval, and a benchmark that grades real served-model runs against a
committed per-engine baseline.

The prebuilt `kx`, both SDKs and the web console move to `0.2.0-rc.1` (PyPI normalizes to
`0.2.0rc1`). Library crates keep their own version lines, governed per-crate by
`cargo-semver-checks`; `kx-proto` is unchanged at `0.12.0`.

Interfaces may still change before 1.0 — pin a commit if you build on it.

### Added

- **Author an App by describing it — derive, review, then approve.** The Apps section is
  one prompt box: `DeriveApp` turns a sentence into a *proposed* App you read and edit
  before anything is created, and the capability menu it may draw from is built from the
  caller's own resolved grants — the model proposes, the runtime decides. Naming a tool is
  not being granted one. (gateway/ui)

- **Capabilities attach to the node that uses them.** Tools, connections, datasets and
  skills are declared on the step in the graph rather than as app-wide side fields, so what
  a node may reach is visible where the work happens, and the DAG is the whole create
  surface. (gateway/ui)

- **An App is a capability another App can call.** A node names another App; at author time
  the callee's blueprint is lowered under its **own** envelope — its own skills, grounding
  and connections — so composing two Apps never widens the caller's reach. (proto/gateway/sdk/ui)

- **Contextual and codified Apps.** An App envelope carries an authoring `mode`. A
  *contextual* App authors markdown only; a *codified* App additionally authors the
  configuration the runtime is orchestrated **from** — `workflow.json` becomes its
  blueprint and `tools.json` its tool wishes, folded onto the envelope on completion. An
  App that sets no mode emits no key, so its canonical bytes, `app_ref` and `app_digest`
  are unchanged. (kx-app/gateway/ui)

- **Hosted Apps talk to the runtime that serves them.** A hosted App installs
  `@kortecx/sdk` from its own gateway — which hosts the package as a scoped npm registry on
  the console listener — and calls the runtime through it, scoped to exactly what its
  envelope declared. (gateway/ui)

- **A cross-run work cache.** Identical deterministic work was deduplicated only *within* a
  run, because a `MoteId` folds a run-scoped graph position. A pure result computed in any
  run can now serve an identical sub-task in another. Off the truth path — a hit proposes
  the cached ref and skips the kernel; a miss is byte-identical to before.
  (kx-worker/kx-work-cache)

- **A graph-RAG retrieval leg (default off).** Ingest extracts `(subject, predicate,
  object)` triples from each chunk into a per-dataset knowledge graph; query fuses a
  multi-hop walk of the query's entity seeds into the existing dense + sparse ranking.
  Behind a flag, so retrieval is byte-identical until you turn it on.
  (kx-dataset-graph/gateway)

- **`bench-v1` — an oracle benchmark over real served-model runs.** The eval harness graded
  a served model only through its trajectory (turns, tools, terminal state), never its
  committed **answer** against a task's expectation. `bench-v1` folds a real run into a
  transcript and scores it with the same oracle the scripted tier uses, ratcheting against
  a committed per-engine baseline — so a capability regression fails a check instead of
  quietly scoring lower. (kx-eval/gateway)

- **Benchmark coverage across ten substrate families.** `bench-v1` spans 26 tasks and three
  invoke shapes: **tool** (picks the right tool; the answer carries a fact only that tool
  could supply) · **react** (an instruction naming a tool it was never granted must fire
  nothing) · **reach** (searches a dataset of sixty-one documents built around near-miss
  distractors, recalls a memory, inherits a capability ceiling) · **swarm** (fan-out →
  gather) · **script** (runs a sandboxed script and answers from what it computed) ·
  **http** (a tool reached over the network under a bearer credential, with pagination) ·
  **failure** (tools that error, hang, and return unusable payloads, plus a healthy
  control) · **menu** (selection from a menu as long as the runtime will present) ·
  **long** (the longest chain the runtime admits) · **adversarial** (input trying to steer
  the agent, including an instruction planted in a tool's output). Each family reports its
  own gate beside the suite-wide ones. An unknown family is a hard error, never a silent
  fall-through — a task driven down the wrong shape still produces a plausible number.
  (kx-eval/gateway/docs)

- **Apps — the durable, shareable unit of agentic capability.** An App is a
  `kortecx.app/v1` envelope that wraps a portable blueprint with by-reference
  context / tool / connection / dataset references, a prompt/rule/skill/memory
  rail, and a steering config (model, max turns, max tool calls) — **plus a
  project**: a tree of markdown files a served model authors into the App's
  content-addressed branch, whose `.md` rides the run's context rail (so a rule in
  `rules/*.md` reaches the model). `kx app new/save/list/get/manifest/run/delete`,
  `export/import/clone`, `scaffold/files/cat/edit/structure`, `lock/unlock` — plus
  the `kx.app(...)` / `app(...)` builders in the Python and TypeScript SDKs and the
  console **Apps** section (a **New App** agentic-creation form and a per-App IDE
  with a file tree + editor, an editable lineage graph, and chat) — author, run,
  schedule, and share them. Bind a cron/webhook trigger to an App with
  `kx triggers add --app <handle>`. `GetAppManifest` and the run preflight report a
  missing **dataset** — the one declared dependency that hard-fails a run. An App
  carries **no authority**: `run` and `import` re-resolve every warrant from the
  caller's own grants, and a shared bundle re-registers connections/secrets by name
  so it resolves each operator's own credentials; the bundle carries the envelope +
  its content closure, **not** the project tree. Off-journal + additive (the
  canonical projection digest is unchanged). (cli/sdk/ui)

- **Hosted (experience) Apps.** A hosted App is a real Vite-React or Next.js project
  the runtime scaffolds into its branch and serves on a loopback dev-server port. It
  ships in the prebuilt binary and every `just serve*` recipe (behind the
  `hosted-apps` cargo feature, wired into the release); **serving one needs Node/npm**
  on the host. The scaffolder carries each sibling's export/prop API forward so files
  agree across the seam, and the supervisor type-checks the project (`tsc --noEmit`)
  before serving — a project that does not compile fails loudly instead of serving a
  blank page. Authored from the console **New App** form or the SDK `.hosted(...)`
  builder (`kx app new` cannot create one). (gateway/ui)

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

- **An authored agentic step now gets a recipe's inference budget.** Every authored step —
  Apps, blueprints, and swarm/chain lowering — was built from the demo warrant with only
  the model id re-pointed, so it kept the demo's **30 s** inference budget while the same
  model under a provisioned recipe gets **120 s**. A slow turn simply failed and the chain
  dead-lettered on turn 0 reporting only "the chain could not progress", naming nothing.
  Every model-facing axis is now re-pointed together, and the budget lives in one constant
  rather than five copies — the duplication is *how* the two paths drifted. Warrants are off
  the journal, so run identity is unchanged. (gateway)

- **A tool that fails to run no longer kills the chain.** An *ungranted* tool was refused
  and re-prompted, so the agent tried something else; a *granted* tool that then failed to
  dispatch — an unreachable MCP server, a capability that cannot serve this run —
  dead-lettered the entire run, and the reason never reached the model, so the agent died
  without being told why. One unusable capability in a wide grant set could end a healthy
  agent. A dispatch failure is now a rejected turn carrying the reason. Exactly-once is
  unaffected: the failed observation is never re-dispatched, and a failed attempt spends its
  tool-call budget, so a permanently broken tool cannot be re-proposed for free.
  (kx-coordinator/kx-journal/kx-model-harness)

- **A hosted App's type-check gate ran on a relative data dir, and failed inverted.**
  Moving the child's working directory before the OS resolved the program path made a
  relative `tsc` vanish, while the probe that decided whether to run it resolved the same
  path against the gateway's own directory and reported it present. Every `--journal
  target/…` serve hit it: a project **with** TypeScript could not start, and one **without**
  it skipped the gate and served unchecked. (gateway)

- **A scaffolded hosted App asks for the SDK version its gateway serves.** The Vite-React
  template declared `"@kortecx/sdk": "^0.1.1"` as a literal. The gateway serves exactly one
  version of that package from its own registry, derived from the SDK's manifest at build
  time, and a caret range on a `0.x` version pins the minor — so the range matched only
  because 0.1.1 happened to be what was being served, and this release's version bump would
  have made every newly scaffolded hosted App's `npm install` unsatisfiable. The template
  now declares the dependency unpinned and the supervisor pins it at write time to the
  version actually being served. (gateway)

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
  + a display-only basis-point score (never an identity input); resolve bytes by
  the exact ref. Exposed in the Python/TypeScript SDKs and an advisory "Discover" mode
  in the Data Lab. (serve/sdk/ui)

### Changed

- **kortecx is fair-code under the Sustainable Use License.** Every manifest, the lockfile
  roots, the docs footer and the third-party notices carry
  `LicenseRef-Kortecx-Sustainable-Use-1.0`. The README is rewritten showcase-first around
  what you can build, and carries a measured benchmark section read off the committed
  `bench-v1` baselines rather than asserted.

- **The documentation describes the runtime that shipped.** The install instructions name
  the paths that work — the TypeScript SDK is served by a running gateway's own scoped npm
  registry, not a public one — and the observability page says where each read actually
  lives now that the console is a flat set of sections. The docs site is built in CI, so its
  broken-link settings finally fire, and three checks the build cannot make (orphan pages,
  anchors into GitHub-hosted files, and the README's benchmark table against the committed
  baselines) are gated too. (docs/ci)

- **An App states the authority rule instead of citing an identifier for it.** User-facing
  copy explains *why* a capability is or is not reachable, in words, where it previously
  pointed at an internal id. (ui)

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
  block (leading-block-only — the fail-closed strict parse and exact-grant
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
