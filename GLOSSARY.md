# Glossary

The vocabulary of the kortecx runtime, for contributors. Each term notes where
it's defined in the code. See [ARCHITECTURE.md](ARCHITECTURE.md) for how the
pieces fit.

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
