# Glossary

The vocabulary of the kortecx runtime, for contributors. Each term notes where
it's defined in the code. See the README's [How it works](README.md#how-it-works)
for how the pieces fit.

### Mote
The unit of work — one step an agent takes (call a model, run logic, hit a tool).
A Mote is *content-addressed*: its identity is derived from its definition plus
its inputs. `kx-mote/src/mote.rs`.

### MoteId
The 32-byte content-addressed identity of a Mote, derived from its definition
hash + input data + position in the graph. Identical work → identical id, which
is what makes "serve the committed result instead of re-running" sound.
`kx-mote/src/id.rs`.

### MoteDef
The author-side *definition* of a Mote: which logic/model/prompt, its tool
contract, its non-determinism class, its effect pattern, etc. The def hash is one
input to the `MoteId`. `kx-mote/src/def.rs`.

### MoteGraph
An author-side container of Motes + their declared parent edges — the shape you
build before submission. The runtime never reads it directly; it folds the
journal. `kx-mote/src/graph.rs`.

### NdClass (non-determinism class)
A three-way tag on every Mote that drives recovery + storage:
**Pure** (deterministic, safe to re-run, droppable/recomputable),
**ReadOnlyNondet** (samples a non-deterministic source but changes no world state;
committed once, served on replay), **WorldMutating** (changes the outside world;
exactly-once, never silently re-run). `kx-mote/src/ndclass.rs`.

### EffectPattern
How a world-mutating Mote is made safe under crashes — its commit protocol:
**IdempotentByConstruction** (the effect carries its own idempotency, safe to
retry), **StageThenCommit** (record intent, then act, then commit — the default),
**ValidateThenCommit** (act, then a critic validates before commit).
`kx-mote/src/effect.rs`.

### EdgeKind / EdgeMeta
Typed dependency edges between Motes. **Data** edges (a parent's output is the
child's input) always cascade on repudiation; **Control** edges (ordering only)
cascade by default but can opt out (`non_cascade`). `kx-mote/src/edge.rs`.

### Journal / JournalEntry
The append-only log that is the single source of truth. Entry kinds include
`Proposed`, `Committed`, `Failed`, `Repudiated`, `EffectStaged`, and run-metadata
facts. The `Journal` trait is a seam (local impl: SQLite).
`kx-journal/src/{lib,entry}.rs`.

### Projection / fold
The read-side: a **pure fold** of the journal into live state (per-Mote status,
the ready set, the dependency index). It is never stored authoritatively — it is
re-derived from the log on restart. "Two folds of the same log prefix produce
equivalent state." `kx-projection/src/{lib,projection}.rs`.

### Ready set
The Motes whose parents are all committed and which are therefore eligible to
run. Computed from the projection; consumed by the scheduler.
`kx-projection` (`ready_set`).

### Commit protocol
The executor-side state machine that drives a Mote's effect to a durable
`Committed` fact according to its `EffectPattern`. The heart of the exactly-once
guarantee. `kx-executor/src/commit_protocol.rs`.

### EffectStaged
A journal fact recording that a world-mutating effect is *about to* happen,
written **before** the effect under `StageThenCommit`. On recovery, an
`EffectStaged` with no matching `Committed` tells the runtime a crash happened in
the window — and an oracle decides whether re-dispatch is safe. `kx-journal`.

### Recovery / re-fold
Restart behavior: re-fold the journal to rebuild the projection, then resume.
Committed steps are *served, not re-run*; in-flight world effects are resumed or
quarantined based on the recovery oracle. `kx-runtime/src/engine.rs`,
`kx-executor/src/lifecycle.rs`.

### Repudiation / cascade
Marking a committed result invalid (operator action, a critic verdict, an
upstream failure). Repudiation **cascades** to dependents along edges (Data always;
Control unless opted out), so nothing downstream trusts an invalidated input.
`kx-projection` (`transitive_consumers`), `kx-journal` (`Repudiated`).

### Warrant / Capability / CapabilityBroker
A **warrant** scopes what a Mote may do (filesystem, network, tools, resources).
A **capability** is a grantable power. The **CapabilityBroker** is the *single
door* through which all world effects pass — enforcement happens here, and it
carries the per-tool idempotency contract. `kx-warrant`, `kx-capability/src/broker.rs`.

### IdempotencyClass
Per-tool declaration of *how* cross-boundary exactly-once is achieved (idempotency
token / deterministic readback / staged intent / at-least-once-with-consent). The
runtime is fail-closed without one. `kx-tool-registry`.

### Critic / CriticVerdict / CheckSpec
A **critic** is a deterministic check on a producer's output. A **CheckSpec**
describes the check as data (schema / dedup / statistical bounds / PII); the
**CriticVerdict** is the content-addressed `Valid`/`Invalid` fact — compared by
exact equality, never a fuzzy score. `kx-critic-types`, `kx-critic`.

### Promotion
A gate that withholds a world-mutating producer's consumers until its declared
critic has committed a `Valid` verdict — fail-closed otherwise. `kx-projection`
(`promotion`).

### Run / instance id / recipe fingerprint
Each submission is a **run** with a fresh, registered, immutable **instance id**
(the cross-boundary idempotency token). The def/content hash survives only as a
**recipe fingerprint** for discovery/reuse — never as run identity. So
re-submitting the same recipe is a *new run*. `kx-journal` (`RunRegistered`).

### ContentRef / ContentStore
Payloads (results, inputs) are stored content-addressed; the journal carries a
32-byte **ContentRef**, not bytes. The **ContentStore** is a seam (local impl:
filesystem). `kx-content/src/lib.rs`.

### Topology shaper / materializer
A Mote that, when committed, *produces the next slice of the graph* (dynamic
fan-out). The **materializer** deterministically derives the child Motes from the
shaper's committed decision, so a re-fold reconstructs the identical children.
`kx-projection/src/materializer.rs`.

### ReAct chain / ReactRound / react seed
The live multi-turn Reason→Act→Observe loop (PR-2d-1, answer-only substrate). A
`react_seed` submit makes the coordinator swap in a run-salted turn-0 model Mote
and anchor a durable `ReactRound` journal fact (turn index, frozen branch, budget
caps — off-DAG, never identity). The settle pass decodes each committed turn via
`kx-toolcall` (the tool-call authority gate) and advances or terminates the chain;
recovery re-derives it from the facts alone. `kx-journal/src/entry.rs`,
`kx-coordinator/src/react_shape.rs`, `kx-toolcall/`.

### Submission refusal
The runtime *refuses unsafe constructions at submit*, before any dispatch — e.g.
a world-mutating Mote with no idempotency-supporting tool, or a run that wasn't
registered first. The refusal vocabulary is one enum; each variant guards a
specific safety invariant. `kx-refusal/src/refusal.rs`.

### Coordinator / Worker (distributed)
The multi-node layer: the **coordinator** is the sole journal writer + worker
registry; **workers** lease ready Motes, dispatch them, and propose results back.
Same guarantees as single-node — distribution is wiring on the same seams, not a
rewrite. Optional for single-node. `kx-coordinator`, `kx-worker`, `kx-proto`.

### Gateway
The networked front door: a gRPC service (`KxGateway`) that hosts an embedded
coordinator + local worker behind bearer-token auth (deny-all by default,
identity derived server-side). It holds a read-only journal handle and proxies
submits to the coordinator — it is **not** a second journal writer. `kx serve`
runs it. `kx-gateway`, `kx-gateway-core`.

### Invoke
The inbound execution path: bind a **published recipe** by handle (e.g.
`kx/recipes/echo`) to JSON args, compile it to a Mote DAG, and run it to a
committed terminal Mote — exactly-once, no new write path. The runtime as a
callable function. `kx-invoke`.

### Blueprint / plan / recipe (terminology)
Three words, three meanings — fixed so they never collide:
**Blueprint** is the *user-facing* name for a reusable, shareable workflow
template (what you pick, fill in, and run from the console/SDKs/CLI).
**plan** is the *agentic topology step* — the planner/shaper's committed
`TopologyDecision` in the live plan/re-plan loop (never a template).
**recipe** is the *frozen wire-legacy* term for a Blueprint: `recipe_fingerprint`,
`ListRecipes`/`GetRecipeForm`, and `kx/recipes/*` handles are durable,
identity-load-bearing wire data and are **never renamed** (old clients +
persisted runs keep working). Display layers say "Blueprint"; the wire says
`recipe`; SDKs export additive `Blueprint*` aliases over the same types.

### Recipe / WorkflowDef
A reusable, parameterized **workflow** that compiles to a Mote DAG (displayed as
a **Blueprint**; `recipe` on the frozen wire). Authored as a `WorkflowDef`
(steps + typed edges + free params), bound to args, then compiled. The shipped
library (`map_reduce`, `fan_out_gather`, `retry_until_critic`, `react_tool_loop`,
`image_batch_describe_reduce`) is composed from pure builders.
`kx-workflow/src/recipes.rs`.

### Prompt template
A pure, fail-closed template (`{name}` slots, `{{`/`}}` escapes) rendered
**before** compile so the final prompt is identity-bearing (it folds into the
`MoteId`). An unfilled slot or unknown param is an error, never silently dropped.
`kx-workflow/src/prompt.rs`.

### Audit event / sink
An **off-the-truth-path** record of the run lifecycle (join-keys only, never
payloads/secrets) written to a best-effort sink (`InMemoryAuditSink`,
`JsonlAuditSink`). Time lives only on the wire DTO, never in the digest — so the
audit trail **never changes** the projection digest. `kx run --audit-log <path>`.
`kx-audit`.

### Signature / Catalog
A **TaskSignature** is the content-addressed, registerable description of a task;
the **catalog** is the sharable registry of signatures, recipes, grants, and
versions, backed by durable SQLite ledgers. Fuzzy-in / exact-out selection is a
single chokepoint. `kx-catalog`.

### Fleet / Team
Append-only **membership facts** + a fail-closed fold that resolve a member's
effective warrant as `intersect(team_warrant, member_role)` — one-level-up
delegation, additive capabilities, nestable fleet-of-teams. Off the journal /
off the trust path. `kx-fleet`.
