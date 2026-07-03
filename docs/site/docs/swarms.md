---
id: swarms
title: Swarms & personas
sidebar_label: Swarms & personas
description: Author multi-agent swarms — N agents running concurrently, each a crash-safe replayable chain, fanned into one gather — with a curated persona library, over one fluent spine (Python · TypeScript · CLI).
---

# Swarms & personas

A **swarm** runs N agents *concurrently*, each as its own crash-safe, deterministically
replayable chain, then merges their committed outputs with a **gather** step. It is pure
composition over the same spine as everything else — `swarm(...)` lowers **byte-identically**
to the equivalent `[a & b] > gather` chain, so no new wire shape, no new step kind, and the
durable guarantees (exactly-once, crash-recovery, warrant-governance) hold for every agent
in the swarm.

A **persona** is a curated, named instruction set (`researcher`, `critic`, `writer`, …) you
attach to an agent. Personas fold into the agent's prompt, so two agents that differ only by
persona are genuinely distinct, replayable steps.

> **One spine.** `swarm` / `team` / `fan_out_gather` / `map_reduce` are methods on the same
> `flow()` builder and top-level `kx.*` factories. Attach an integration and it becomes an
> [App](./apps.md); everything lowers through the one `SubmitWorkflow` / `RunApp` path.

## A swarm of personas

<!-- prettier-ignore -->
```python
import kortecx as kx

# Three roles work the shared goal in parallel; a lead synthesizes their outputs.
result = kx.swarm(
    kx.persona("researcher"),
    kx.persona("critic"),
    kx.persona("writer"),
    goal="Write a briefing on durable execution",
).run()
print(result.text)
```

```typescript
import * as kx from "@kortecx/sdk";

const result = await kx
  .swarm([kx.persona("researcher"), kx.persona("critic"), kx.persona("writer")],
         { goal: "Write a briefing on durable execution" })
  .run();
```

```bash
# The string DSL authors the same topology: two agentic leaves → a gather.
kx chain "[a@web-search & b@web-search] > g" \
  --task 'a={"kind":"model","prompt":"Research angle A"}' \
  --task 'b={"kind":"model","prompt":"Research angle B"}' \
  --task 'g={"kind":"model","prompt":"Synthesize the findings"}' \
  --wait
```

Each participant may be a prompt, a `(prompt, tools)` pair, an `Agent` / `persona`, or a
`Flow`. Give a participant tools and it becomes a bounded reason→tool→observe agent:

```python
analyst = kx.persona("analyst", tools=["web-search@1"])
kx.swarm(analyst, kx.persona("skeptic"), goal="Is the Q3 forecast credible?").run()
```

## The gather

By default the gather is a **model synthesizer** that reads every participant's committed
output (injected as its data-edge parents) and writes one coherent answer. Steer it, or fold
deterministically instead:

```python
kx.swarm(a, b, c, gather="Merge into a single ranked list")   # a custom synthesis prompt
kx.swarm(a, b, c, synthesize=False)                            # a PURE deterministic fold
kx.team(a, b, c, goal="…")                                     # a swarm with a lead (always synthesizes)
```

`fan_out_gather(*samplers)` (sample N ways, combine) and `map_reduce(*mappers, reduce=…)`
are the same fan-in family with plain (non-persona) branches.

## Reusable agents

An `Agent` is a reusable named step; bind it to a task with `.on(...)` (an alias of
`.as_flow(...)`):

```python
researcher = kx.agent("You are a meticulous researcher.", tools=["web-search@1"])
kx.flow().parallel(researcher.on("topic A"),
                   researcher.on("topic B")).then("Synthesize").run()
```

## A swarm that acts — integrations in an App

When a swarm needs a **credentialed connector** (Gmail, Discord, …), name it an **App** with
`.as_app(name)` — that is the explicit boundary where connections and secret scope attach.
Running an App routes through `RunApp`, so the server resolves your registered connection and
narrows the run warrant's secret scope (the value never travels, D81):

```python
# kx connections add --provider gmail   (register the connector once)
(kx.flow()
   .agent("Draft and send a summary reply", tools=["kx-connector-gmail/send"])
   .as_app("mailer").with_gmail().secrets(["KX_GMAIL_CREDENTIAL"])
   .run(args={"to": "team@example.com"}))
```

```typescript
await kx.flow()
  .agent("Draft and send a summary reply", { tools: ["kx-connector-gmail/send"] })
  .asApp("mailer").withGmail().secrets(["KX_GMAIL_CREDENTIAL"])
  .run({ to: "team@example.com" });
```

`with_discord()` is the Discord equivalent. See [Apps](./apps.md) for the full envelope and
[Authoring a connector](./authoring-a-connector.md) for `kx new connector` (scaffold your own
sidecar).

## What holds

- **Concurrent, not sequential** — each agent is an independent chain that commits on its own;
  the gather fires once all have committed.
- **Crash-safe + replayable** — kill a swarm mid-run and recovery re-derives byte-identical
  agent identities; the projection digest is unchanged.
- **Governed** — the server compiles and warrants every agent (SN-8); the client only proposes
  topology. Personas and swarm sugar change *what is proposed*, never authority.
- **One lowering** — `swarm(...).to_chain()` / `.to_blueprint()` round-trips through the same
  `DagSpec` the string DSL and the visual builder produce; the Python, TypeScript, and CLI
  surfaces lower it byte-identically (the golden tri-surface contract).
