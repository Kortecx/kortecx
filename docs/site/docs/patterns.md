---
id: patterns
title: Orchestration patterns
sidebar_label: Orchestration patterns
description: Author multi-agent orchestration — a hierarchical supervisor, best-of-N or exact-majority consensus, and iterative review loops — each pure composition over one fluent spine, lowered byte-identically across Python · TypeScript · CLI.
---

# Orchestration patterns

Where a [swarm](./swarms.md) fans N agents out and merges them, the **orchestration patterns**
add *structure* on top of the same spine: a **supervisor** plans and delegates, a **consensus**
selects or votes on the best answer, and a **review loop** iteratively refines a draft. Each is
pure composition — it lowers **byte-identically** to an ordinary `p > [a & b] > g` chain, so no
new wire shape, no new step kind, and the durable guarantees (exactly-once, crash-recovery,
per-step warrants) hold for every agent.

Each pattern is a method on the same `flow()` builder and a top-level `kx.*` factory, and each is
reachable from the `kx swarm` CLI verb and the visual builder — the one authoring surface, three
ways in.

> **One spine.** `supervisor` / `consensus` / `review_loop` compose the same fan-out/fan-in
> primitives as `swarm`. Attach an integration and it becomes an [App](./apps.md); everything
> lowers through the one `SubmitWorkflow` / `RunApp` path, and the Python, TypeScript, and CLI
> surfaces lower it identically (the golden tri-surface contract).

## Supervisor — plan, delegate, integrate

A **supervisor** is a hierarchy: a lead **planner** decomposes the goal, the **workers** each act
on that plan in parallel, then the lead **integrates** their outputs — the topology
`planner > [workers] > gather`.

**Python** — `supervisor(*workers, planner=…, goal=…, gather=…)`:
```python
import kortecx as kx

result = kx.supervisor(
    kx.persona("researcher"),
    kx.persona("writer"),
    planner="Plan a briefing on durable execution",
    goal="Cover crash-recovery and exactly-once",
).run()
print(result.text)
```

**TypeScript** — `supervisor(workers, { planner, goal, gather })`:
```typescript
import * as kx from "@kortecx/sdk";

const result = await kx
  .supervisor([kx.persona("researcher"), kx.persona("writer")],
              { planner: "Plan a briefing on durable execution",
                goal: "Cover crash-recovery and exactly-once" })
  .run();
```

**CLI** — `kx swarm --pattern supervisor`:
```bash
kx swarm "Research crash-recovery" "Write the briefing" \
  --pattern supervisor \
  --planner "Plan a briefing on durable execution" \
  --goal "Cover exactly-once" --wait
```

The planner's committed output is a data-edge parent of every worker (they run *on* the plan);
every worker feeds the `gather` lead (default: a model integrator — steer it with `gather=…`, or
`synthesize=False` for a PURE deterministic fold).

:::note Static hierarchy today
This supervisor is **static-hierarchical**: a fixed team, authored up front. `rounds` / `pool`
are reserved for the runtime **topology shaper** (a planner that decides team size and re-plans
each round); they sit in the signature so the API is stable when the shaper wires them, but
passing `rounds > 1` or `pool` raises today rather than silently ignoring it. Local worker
concurrency is set by the server worker pool (`kx serve --workers` / `KX_WORKERS`).
:::

## Consensus — judge or vote

A **consensus** runs N voters in parallel, then reduces to one answer — the topology
`[v1 & v2 & …] > reduce`. Two reduce modes:

- **`vote="judge"`** (default) — a model **judge** reads the candidates and SELECTS the single
  best one (distinct from a swarm's *merge*). Steer it with `judge=…`.
- **`vote="majority"`** — the server reduces to the **exact-equality plurality**: the
  most-frequent voter output by EXACT byte-equality, ties broken by first appearance. No model
  call — it is a deterministic fold, best for constrained / classification-style outputs.

**Python** — `consensus(*voters, vote=…, goal=…, judge=…)`:
```python
# judge: a model picks the best of N reasoned answers
kx.consensus(
    kx.persona("analyst"),
    kx.persona("skeptic"),
    kx.persona("engineer"),
    goal="Is this design sound?",
    vote="judge",
).run()

# majority: exact-equality plurality over constrained answers
kx.consensus(
    "Answer only 'yes' or 'no'.",
    "Answer only 'yes' or 'no'.",
    "Answer only 'yes' or 'no'.",
    goal="Is the Q3 forecast credible?",
    vote="majority",
).run()
```

**TypeScript** — `consensus(voters, { vote, goal, judge })`:
```typescript
await kx.consensus([kx.persona("analyst"), kx.persona("skeptic"), kx.persona("engineer")],
                   { goal: "Is this design sound?", vote: "judge" }).run();
```

**CLI** — `kx swarm --pattern consensus --vote judge|majority`:
```bash
kx swarm "Argue for" "Argue against" "Weigh both" \
  --pattern consensus --vote judge --goal "Is this design sound?" --wait
```

:::info Exact equality, never similarity
The majority reducer decides by **exact byte-equality** — never a similarity score (SN-8). That is
what makes it a durable, replayable fact: the same voters always fold to the same winner.
:::

## Review loop — draft, then refine

A **review loop** is iterative refinement: a `worker` drafts, then a `reviewer`
reviews-and-improves the draft `rounds` times — the sequential chain
`worker > review > review > …`. Each pass reads the previous version (its data-edge parent) and
emits a better one; the last step's output is the result.

**Python** — `review_loop(worker, *, reviewer=…, rounds=1, goal=…)`:
```python
kx.review_loop(
    "Draft a launch email for the durable-execution release",
    reviewer="Tighten it, fix errors, and cut fluff",
    rounds=2,
).run()
```

**TypeScript** — `reviewLoop(worker, { reviewer, rounds, goal })`:
```typescript
await kx.reviewLoop("Draft a launch email for the durable-execution release",
                    { reviewer: "Tighten it, fix errors, and cut fluff", rounds: 2 }).run();
```

This is the author-static refine loop; a runtime-adaptive "revise until a critic passes" loop is
the topology-shaper follow-on.

## The `kx swarm` verb

`kx swarm` authors any of these patterns from bare agent prompts — no chain DSL to hand-write.
Each positional is one agent prompt; `--pattern` picks the topology; `--goal` is appended to every
participant's prompt.

```bash
kx swarm "<agent prompt>"... [--pattern swarm|supervisor|consensus] [--planner <p>]
         [--gather <p>] [--vote judge|majority] [--goal <g>] [--seed N] [--wait] [--dry-run]
```

| Pattern | Topology | Key flags |
|---|---|---|
| `swarm` (default) | `[a0 & a1 & …] > gather` | `--gather` |
| `supervisor` | `planner > [a0 & a1 & …] > gather` | `--planner`, `--gather` |
| `consensus --vote judge` | `[a0 & a1 & …] > judge` | `--gather` (the judge prompt) |
| `consensus --vote majority` | `[a0 & a1 & …] > reduce` | — (server-reduced) |

`--dry-run` lowers and validates without submitting (needs no gateway) — the offline check that
the topology is what you meant.

## Authoring in the console

The visual [blueprint builder](./blueprint-builder.md) has a **`+ Swarm`**, **`+ Supervisor`**,
**`+ Consensus · judge`**, and **`+ Consensus · majority`** button. Each drops a pre-wired cluster
of ordinary agent / pure nodes onto the canvas — the same DAG the SDK and CLI author — which you
then fill in per node and run. The pattern is just a scaffold of the nodes you already know.

## What holds

- **Structure without new primitives** — supervisor / consensus / review-loop are compositions of
  the same fan-out, fan-in, and sequential edges as every other chain; no new step kind, no new
  wire shape.
- **Crash-safe + replayable** — kill an orchestration mid-run and recovery re-derives
  byte-identical agent identities; the projection digest is unchanged.
- **Governed** — the server compiles and warrants every agent (SN-8); the client only proposes
  topology. The majority reducer decides by exact byte-equality, never similarity.
- **One lowering** — the Python, TypeScript, CLI, and visual-builder surfaces lower each pattern
  to the same `DagSpec` (the golden tri-surface contract). See [Swarms & personas](./swarms.md)
  for the fan-out family and [the DSL reference](./chains/dsl-reference.md) for the string form.
