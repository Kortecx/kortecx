---
id: concepts
title: Concepts
sidebar_label: Concepts
description: The core vocabulary of the Kortecx runtime — Mote, Journal, Projection, Warrant, Chain.
---

# Concepts

The vocabulary that the rest of the docs assumes. Each term maps to a real
construct in the runtime; the full contributor glossary lives in
[`GLOSSARY.md`](https://github.com/Kortecx/kortecx/blob/main/GLOSSARY.md).

## Mote

The **unit of work** — one step an agent takes (call a model, run logic, hit a
tool). A Mote is *content-addressed*: its identity is derived from its
definition plus its inputs. Identical work yields an identical identity, which is
what makes "serve the committed result instead of re-running" sound.

A Mote also carries a **non-determinism class** that drives recovery:

- **Pure** — deterministic, safe to re-run, recomputable.
- **ReadOnlyNondet** — samples a non-deterministic source but changes no world
  state; committed once, then served on replay.
- **WorldMutating** — changes the outside world; exactly-once, never silently
  re-run.

## MoteId

The 32-byte content-addressed **identity** of a Mote, derived from its definition
hash + input data + position in the graph. Identities are always computed by the
runtime — the SDKs and CLI carry the server's bytes (as lowercase hex) but never
construct one. This is a load-bearing security property (see
[Security](./security.md#identity-is-server-derived)).

## Journal / JournalEntry

The **append-only log** that is the single source of truth. Entry kinds include
`Proposed`, `Committed`, `Failed`, `Repudiated`, `EffectStaged`, and run-metadata
facts. The `Journal` is a trait seam; the local implementation is SQLite.

Nothing downstream is trusted until it is a `Committed` fact in the journal.

## Projection / fold

The **read side**: a *pure fold* of the journal into live state (per-Mote status,
the ready set, the dependency index). The projection is never stored
authoritatively — it is re-derived from the log on every restart. The invariant:
*two folds of the same log prefix produce equivalent state.*

The runtime exposes a deterministic **projection digest** over a run, which is
what the [exactly-once demo](./quickstart.md#prove-exactly-once) asserts is
identical across a clean run and a crash-then-replay run.

## Ready set

The Motes whose parents are all `Committed`, and which are therefore eligible to
run. Computed from the projection; consumed by the scheduler.

## Recovery / re-fold

Restart behavior: re-fold the journal to rebuild the projection, then resume.
Committed steps are *served, not re-run*; in-flight world effects are resumed or
quarantined based on a recovery oracle.

## Warrant / Capability / CapabilityBroker

A **warrant** scopes what a Mote may do — filesystem, network, tools, resources.
A **capability** is a grantable power. The **CapabilityBroker** is the *single
door* through which all world effects pass; enforcement happens there, and it
carries the per-tool idempotency contract.

Warrants are **built server-side**. A model can *propose* an action, but only the
runtime's checks — exact-equality, never a fuzzy score — can let it happen. See
[Security](./security.md).

## Critic / Promotion

A **critic** is a deterministic check on a producer's output, described as data
(schema / dedup / statistical bounds / PII). Its verdict is a content-addressed
`Valid` / `Invalid` fact, compared by exact equality. **Promotion** is the gate
that withholds a world-mutating producer's consumers until its declared critic
has committed a `Valid` verdict — fail-closed otherwise.

## Run / instance id / recipe fingerprint

Each submission is a **run** with a fresh, registered, immutable **instance id**
(the cross-boundary idempotency token). The definition/content hash survives only
as a **recipe fingerprint** for discovery and reuse — never as run identity. So
re-submitting the same recipe is a *new run*.

## ContentRef / ContentStore

Payloads (results, inputs) are stored content-addressed. The journal carries a
32-byte **ContentRef**, not bytes. The **ContentStore** is a seam; the local
implementation is the filesystem.

## Blueprint / plan / recipe

Three words, three fixed meanings:

- **Blueprint** — the *user-facing* name for a reusable, shareable workflow
  template (what you pick, fill in, and run from the console / SDKs / CLI).
- **plan** — the *agentic topology step*: the planner/shaper's committed
  `TopologyDecision` in the live plan / re-plan loop (never a template).
- **recipe** — the *frozen wire term* for a Blueprint. `recipe_fingerprint`,
  `ListRecipes`, and `kx/recipes/*` handles are durable, identity-load-bearing
  wire data and are **never renamed**. Display layers say "Blueprint"; the wire
  says `recipe`.

## Chain

A **chain** composes task handles into a DAG using a small string DSL —
`a > b` (sequential), `a & b` / `a | b` (parallel), `[ … ]` (grouping). It is the
authoring front door for building a Blueprint's topology, and lowers to a
canonical `(steps, edges)` form that is **byte-identical** across the Python,
TypeScript, and CLI implementations.

A chain describes *topology only*. The server still compiles and warrants every
step — a chain only changes what is **proposed**. Full grammar and worked
examples: [Chains DSL reference](./chains/dsl-reference.md).

## ReAct chain / ReactRound

The live multi-turn **Reason → Act → Observe** loop. A run can drive a model
through turns where it reasons, calls a real MCP tool, observes the result, and
answers — with **every turn committed as a durable fact**. Crash the server
mid-loop and it resumes from its committed turns. Reach it via the
`kx/recipes/react` Blueprint (see the [Quickstart](./quickstart.md#run-the-agent-loop)).

## Gateway / Invoke

The **gateway** is the networked front door: a gRPC service (`KxGateway`) that
hosts an embedded coordinator + local worker behind bearer-token auth (deny-all
by default, identity derived server-side). `kx serve` runs it.

**Invoke** is the inbound execution path: bind a published Blueprint by handle
(e.g. `kx/recipes/echo`) to JSON args, compile it to a Mote DAG, and run it to a
committed terminal Mote — exactly-once, the runtime as a callable function.
