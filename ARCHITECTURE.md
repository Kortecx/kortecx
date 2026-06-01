# Architecture

A map of the kortecx runtime for contributors. Read [README.md](README.md) first
for *what* kortecx is and *why*; this document is *how it's built* — the crates,
how they stack, how a step flows through the system, and where the extension
points are. Pair it with [GLOSSARY.md](GLOSSARY.md) for the vocabulary.

> Status: early development. Internals are real and tested; interfaces will move
> before 1.0.

## The one-paragraph model

kortecx is an **execution kernel** for AI agents. The unit of work is a **Mote** —
one step, content-addressed by its definition + inputs, so identical work has an
identical identity. The single source of truth is an append-only **journal**: the
runtime never holds authoritative state in memory, it *appends facts* (proposed,
committed, failed, …) to the log. All live state is a **projection** — a pure
*fold* of the journal, re-derived from scratch on restart. That is what makes the
runtime durable: a crash loses no truth, because the truth is the log, and
recovery is just folding it again. A step that changes the outside world is
driven through a **commit protocol** that records intent before it acts, so the
effect lands **exactly once** even across crashes, retries, and redistribution.

Everything else — scheduling, capability enforcement, inference, distribution —
is a layer around that journal/projection/commit spine.

## Crate map

29 crates, a clean layered DAG (no cycles). The foundation is a narrow waist that
almost everything depends on; the engine and the distributed layer stack cleanly
on top. Legend:

- **core** — the single-node guarantee path. To understand the runtime, read these.
- **engine** — scheduling/execution/inference built on core.
- **distributed (P2/P3)** — the multi-node layer. *Not needed to run or understand
  single-node.* Distribution is wiring on top of the same seams, not a rewrite.
- **forward-seam** — built ahead of its consumer; off the guarantee path. Safe to
  skip on a first read.
- **ffi / harness** — native inference bindings and the test instrument.

```
                          kx-critic-types ── kx-llamacpp-sys        (leaves)
                                 │                  │
   ┌──────────── kx-mote ────────┘            kx-llamacpp           [ffi]
   │   (Mote, MoteId, MoteDef, NdClass, EdgeMeta, …)
   │      │
   │   kx-content ── ContentStore + ContentRef (content-addressed bytes)
   │      │
   │   kx-journal ── Journal + JournalEntry (the append-only log of facts)   [core]
   │      │
   │   kx-warrant ── capabilities, roles, scopes (what a Mote may do)        [core]
   │      │
   │   kx-projection ── the pure fold: log → live state (ready set, cascade) [core]
   │
   ├── kx-capability ── CapabilityBroker (the ONLY door to world effects)    [core]
   ├── kx-tool-registry ── tool resolution + IdempotencyClass
   ├── kx-context-assembler ── builds a Mote's input context (model menu)
   ├── kx-model-validator / kx-inference / kx-llamacpp ── the model seam     [engine]
   ├── kx-critic-types / kx-critic ── deterministic verdicts (the exit gate)
   │
   ├── kx-scheduler ── picks the ready set from the projection               [engine]
   ├── kx-executor ── the Mote lifecycle + commit protocols + recovery       [engine]
   └── kx-runtime ── the single-node engine + the `run`/`replay` binary      [engine]

   distributed (P2/P3 — optional for single-node):
     kx-proto (gRPC schema) → kx-coordinator (sole journal writer, worker
     registry) → kx-worker (leases work, dispatches, proposes back) ;
     kx-chaos (seed-deterministic kill-and-replay harness)

   forward-seams (off the guarantee path):
     kx-capture (step capture), kx-dataset (typed committed data + retrieval),
     kx-memoizer (exact-equality cache), kx-tiering (storage tiering),
     kx-normalizer (input canonicalization)

   harness: kx-model-harness (drives a real GGUF through the runtime via the seams)
```

The waist — `kx-mote` → `kx-content` → `kx-journal` → `kx-warrant` → `kx-projection`
— is where the load-bearing invariants live. Changes there ripple widely; changes
in a leaf or forward-seam are local. **If you're new, start by reading a leaf or
an example, not the executor.**

## How a step flows (the lifecycle)

```
  submit ─► register run (immutable instance id)
         ─► submit Mote ─► REFUSAL GATE (refuse unsafe constructions up front)
         ─► journal.append(Proposed)
                 │
   scheduler ◄───┴── reads ready_set from the projection fold
         │           (a Mote is ready when its parents are committed)
         ▼
   executor ── dispatches under a commit protocol:
         │       • IdempotentByConstruction → effect → append(Committed)
         │       • StageThenCommit          → append(EffectStaged) → effect → append(Committed)
         │       • ValidateThenCommit        → effect → critic verdict → append(Committed|Repudiated)
         │     (world effects go ONLY through the CapabilityBroker)
         ▼
   journal ── append(Committed)  ─►  projection folds it  ─►  consumers unblock
```

**Recovery** is the same machinery in reverse: on restart the runtime re-folds the
journal. If it sees an `EffectStaged` with no matching `Committed`, an oracle
decides whether the effect is safe to re-dispatch (a pre-commit crash) or must be
quarantined (a terminal failure) — that is how exactly-once survives a crash in
the middle of a world-mutating step. Try it with the demo in the README.

## The trait seams (extension + deployment boundaries)

Six traits are the load-bearing seams. The same trait is implemented one way for
the local single-node stack and (later) another way for the hosted/distributed
deployment — the trait *shape* is the boundary, so distribution and cloud are new
implementations, not a rewrite of the engine.

| Seam | Trait | Defined in | Abstracts |
|---|---|---|---|
| Journal | `Journal` | `kx-journal/src/lib.rs` | the append-only log of facts (local: SQLite) |
| Content store | `ContentStore` | `kx-content/src/lib.rs` | content-addressed bytes (local: filesystem) |
| Capability broker | `CapabilityBroker` | `kx-capability/src/broker.rs` | the single door to world effects + idempotency |
| Resource manager | `ResourceManager` | `kx-executor/src/resource_manager.rs` | admission/slots for dispatch |
| Inference backend | `InferenceBackend` | `kx-inference/src/backend.rs` | model inference (local: llama.cpp) |
| Worker registry | `WorkerRegistry` | `kx-coordinator/src/registry.rs` | worker liveness (distributed only) |

(`MoteExecutor` in `kx-executor/src/executor_trait.rs` is a seventh, engine-internal
seam for how a Mote body runs.)

## Identity, durability, and determinism

- **Identity is content-addressed.** A `MoteId` is derived from the Mote's
  definition + its inputs, so re-submitting the same recipe with the same inputs
  yields the same id — that is what makes deduplication and "serve, don't re-run"
  sound. (Each *run* also gets a fresh registered instance id; see GLOSSARY.)
- **Durability/resumability is the headline guarantee.** Register the run; if a
  Mote fails midway, the journal + recovery fold let it resume or compensate —
  exactly-once across the failure, never a silent double-fire.
- **Byte-for-byte replay is a sub-case, not the headline.** For pure/read-only
  steps, a re-fold reproduces an identical projection digest; for world-mutating
  steps the guarantee is *exactly-once*, not bit-equality. (CI's byte-determinism
  gate is about reproducible *builds*, a separate, supply-chain concern.)

## Where to look

| You want to understand… | Read |
|---|---|
| **How to author + run a workflow** | `cargo run -p kx-workflow --example author_a_workflow` |
| The unit of work + identity | `kx-mote/src/{mote,id,def,ndclass,edge}.rs` |
| The log + its entry kinds | `kx-journal/src/{lib,entry}.rs` |
| Live state / the fold / recovery | `kx-projection/src/{lib,projection,state}.rs` |
| The lifecycle + commit protocols | `kx-executor/src/{lifecycle,commit_protocol}.rs` |
| What gets refused at submit, and why | `kx-refusal/src/refusal.rs` |
| The single-node engine + CLI | `kx-runtime/src/{engine,main}.rs` |
| A minimal Mote body | `kx-executor/examples/` |

For the deeper *design rationale* (the "why" behind specific invariants and
decisions), the public doc-comments on the core types are the source of truth for
contributors; the project's full design corpus is maintained privately. If a
public doc-comment is unclear or references something you can't find, that's a
documentation bug worth an issue — see [CONTRIBUTING.md](CONTRIBUTING.md).
