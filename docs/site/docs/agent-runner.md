---
id: agent-runner
title: Agents & reasoning
sidebar_label: Agents & reasoning
description: Compose chains of agents, control a model's reasoning, and read a run's reasoning back — across the builder, the SDKs, and the console.
---

# Agents & reasoning

An **agent** is a MODEL step: a prompt routed to the served model. You compose
agents into workflows (a **chain of agents**), control how much a model **reasons**,
and read that reasoning back from a run. The live tool-using loop (plan / re-plan /
critic / ReAct turns) runs **inside `kx serve`** and is crash-safe end to end.

## Chains of agents

Wire one agent's output into the next with a **data edge** — the upstream result
becomes the downstream agent's context. Author it visually in the
[Blueprint builder](./blueprint-builder.md#chained-agents), or in one line:

```python
from kortecx.chains import model, chain

c = chain("research > summarize", {
    "research":  model("kx-serve:qwen3-4b-q4_k_m", "Research the question thoroughly."),
    "summarize": model("kx-serve:qwen3-4b-q4_k_m", "Summarize the research into 3 bullets."),
})
run = await client.run_chain(c, wait=True)
```

Each agent is a normal Mote: its result is committed content-addressed (see
[Reading run outputs](./reading-run-outputs.md)) and durable across a crash.

## Reasoning mode

Reasoning-capable models (e.g. Qwen3) can emit a `<think>…</think>` block before
their answer. Kortecx exposes an **opt-in, declared** reasoning control — never a
silent default:

| Mode | Effect |
|---|---|
| *(unset)* | the model's own default behavior — **byte-identical** to a step with no reasoning param |
| `full` | native `/think` — full reasoning |
| `minimal` | `/think` with a brief-reasoning hint |
| `off` | native `/no_think` (+ a defensive strip of any leftover `<think>`) |

Set it **per agent step** in the builder's config drawer, or as a step param:

```python
b.add_step(StepInput(
    kind="model", model_id="kx-serve:m", prompt="Explain the trade-off.",
    params={"reasoning": "minimal"},
))
```

Because the mode rides in the step's content-addressed params, a **set value yields a
new, honest step identity** (a different computation), while an **unset** value leaves
the default semantics — and the content-addressing — unchanged. The control selects
*how the model behaves*; it never fabricates or hides a committed result.

## Reading reasoning back

A model's reasoning is **already durable** — the `<think>` block rides in the turn's
committed result bytes, and the agent loop's step facts are journaled (`ReactRound`,
folded into `capture.db`). The console therefore treats reasoning as a **display**
concern:

- In **Chat**, an assistant reply's leading `<think>` block is split into a
  collapsible **Reasoning** disclosure above the answer. The **Show reasoning** toggle
  (Settings) hides or shows that disclosure — the answer always renders.
- The live **DAG-of-thought** (the run's Motes) is a separate **Show DAG-of-thought**
  toggle.

Neither toggle can gate capture — they are pure presentation over facts the runtime
already committed.

## The live tool loop

When an agent is granted tools, the loop **plans** topology, **re-plans** on failure,
passes **critic** gates, and runs **ReAct turns** against real MCP tools — every turn
a durable fact, bounded by `max_turns` / `max_tool_calls`. Crash the server mid-loop
and it resumes from its committed turns. Inspect a run's turns with `ListReactTurns`
(`kx react list`). See the
[Quickstart agent loop](./quickstart.md#run-the-agent-loop) and
[Concepts → ReAct chain](./concepts.md#react-chain--reactround).
