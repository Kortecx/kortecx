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

## Run an agent (the agent-runner)

The fastest way to put the loop to work: give a **goal**, get back a reasoned
**answer** plus the **audited set of actions** the agent took. `run_agent` is a thin,
permission-gated wrapper over the live ReAct recipe — the runtime **derives the
warrant** (you never author one, SN-8) and runs the bounded reason → tool → observe
loop, then returns the committed answer with the tools it fired. It never uses
`SubmitRun` (admission is identical to `kx invoke`).

**Python** — `run_agent(goal, *, context=…, inputs=…, wait=True)`:

```python
import kortecx as kx

result = kx.run_agent("Use the echo tool to repeat 'pong'.")
print(result.answer)                        # the reasoned final answer
for a in result.actions:                    # the audited action set
    print(f"  turn {a.turn}: {a.tool_id}@{a.tool_version}")
# async: await kx.run_agent_async(goal, client=async_client)
```

**TypeScript** — `runAgent({ goal, context?, inputs?, wait? })`:

```ts
import { runAgent } from "@kortecx/sdk";

const result = await runAgent({ goal: "Use the echo tool to repeat 'pong'." });
console.log(result.answer);
for (const a of result.actions) {
  console.log(`  turn ${a.turn}: ${a.toolId}@${a.toolVersion}`);
}
```

**CLI** — `kx agent run`:

```bash
kx agent run --goal "Use the echo tool to repeat 'pong'." --json
# { "answer": "...", "actions": [{ "tool_id": "mcp-echo/echo", "tool_version": "1", "turn": 1 }],
#   "run_handle": "<hex>", "instance_id": "<hex>" }
# exit 0 = answered · 1 = the run failed · 3 = timed out (resume with `kx react list`)
```

- **`context`** attaches published [context bundles](./context.md) the server resolves
  and injects (identity-bearing — a different context is a different run).
- **`inputs`** (`k=v` on the CLI, a map in the SDKs) fold into the goal prompt.
- The returned **`AgentResult`** carries `answer` (+ `answer_bytes`), `actions` (the
  audited tool set), and the re-attachable `run_handle` / `instance_id`. With
  `wait=False` you get the run handle back instead and assemble the result later.

> **Composing in a chain.** The agent-runner is the *steered* whole-run entry (the
> model picks tools turn by turn). To put a tool-using agent *step* inside a larger
> DAG, use the deterministic lane — `flow().agent(prompt, tools=[…])` or the
> `model@tool` chain step (see the [DSL reference](./chains/dsl-reference.md)) — where
> the granted tool set is fixed and part of the step's identity. There is no separate
> chains `agent()` node by design (it would be a second, divergent wire shape).

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
(`kx react list`). Each agent run on a server is its **own chain** even though a server
shares one run identity across calls, so `run_agent` returns the answer + actions for
*that* call (scope `kx react list` to one chain with `--chain <key>`, or pass the
`react_chain_salt` the SDKs return). The same loop powers the `Agent(tools=[fn])`
one-liner (Python / TS)
and the `model@tool` chain step — the granted tool set is part of the step's identity,
so the loop replays deterministically (and a model's bare/leaf tool name resolves to the
granted `&lt;server&gt;/&lt;name&gt;`). See the
[Quickstart agent loop](./quickstart.md#run-the-agent-loop) and
[Concepts → ReAct chain](./concepts.md#react-chain--reactround).

## Graceful tool-call recovery

A real model occasionally proposes a tool call the runtime can't honor — an
**ungranted name**, **arguments that don't match the tool's schema**, or a
**malformed proposal**. The loop does **not** die on the first such mistake.
Instead the turn settles as **`rejected`** (a non-terminal branch), the
fail-closed reason is fed back to the model on the next turn, and the model
**self-corrects** — fixing its arguments, picking a tool it was actually granted,
or answering directly. Each rejected attempt spends one tool-call from the
budget, so the recovery is bounded: when `max_turns` / `max_tool_calls` is
exhausted with no answer, the chain **dead-letters loudly** (never a silent wedge,
never a fabricated answer).

Two things make the model more likely to get it right the first time:

- The tool menu the model sees includes a well-formed **`Example:`** call for
  each tool (the exact JSON keys + a typed placeholder), so it emits the right
  shape.
- Common, unambiguous JSON malformations in the **arguments** — a trailing comma,
  an unquoted key, a single-quoted string (the JSON5-ish subset real models emit)
  — are repaired when validating; the authority gate (which tool, which grant)
  stays exact, only the argument *syntax* is forgiven.

### Different models, different shapes

Tool-calling output varies by model, so the runtime recognizes the common
**call envelopes** — accept-side and fail-closed — and routes them all through the
same exact grant check:

| Model family | Shape recognized |
|---|---|
| Kortecx / generic JSON | `{"tool_call":{"name":…,"args":…}}` |
| Gemma | `<\|tool_call>call:NAME{…}<tool_call\|>` |
| Llama 3.x | `<\|python_tag\|>{"name":…,"parameters":…}` |
| Qwen / Hermes | `<tool_call>{"name":…,"arguments":…}</tool_call>` |
| OpenAI / Hermes (markerless) | `{"name":…,"arguments":{…}}` |
| OpenAI (plural wrapper) | `{"tool_calls":[ {"name":…,"arguments":{…}} ]}` |

The arguments bag is accepted under `args`, `arguments`, or `parameters`, as either
a JSON object or a pre-serialized JSON string. A reasoning preamble
(`<think>…</think>` / Gemma `<|channel>…`) or a Markdown code fence around the call
is stripped first. Anything the runtime doesn't recognize as a call is treated as a
normal answer (it never mis-fires a tool). This is **acceptance** only — the tool
name still resolves to an exact grant (SN-8); a model can never widen its own
authority by how it phrases a call. The two **markerless** shapes carry no commitment
marker, so they fire only when the name resolves to a granted tool *and* an explicit
arguments bag is present — otherwise the output is a normal answer, never a
false-positive refusal. A `tool_calls` array with **more than one** call is not yet
run in a single turn (one tool fact per turn); it degrades to a normal answer rather
than silently dropping calls.

Inspect what happened per turn:

```bash
kx react list --instance <id>        # turn 1  branch rejected  reason <why> …
                                     # turn 2  branch answer
```

The reason is also on `ReactTurn.rejection_reason` (Python / TypeScript SDK) and
expands inline on the rejected chip in the console's agent-loop strip.
