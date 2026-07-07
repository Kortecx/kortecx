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

result = kx.run_agent("Use the echo tool to repeat 'pong'.", max_tool_calls=20)
print(result.answer)                        # the reasoned final answer
for a in result.actions:                    # the audited action set (one per fired call)
    print(f"  turn {a.turn}.{a.call_index}: {a.tool_id}@{a.tool_version}")
# async: await kx.run_agent_async(goal, client=async_client)
```

**TypeScript** — `runAgent({ goal, context?, inputs?, maxToolCalls?, wait? })`:

```ts
import { runAgent } from "@kortecx/sdk";

const result = await runAgent({ goal: "Use the echo tool to repeat 'pong'." });
console.log(result.answer);
for (const a of result.actions) {
  console.log(`  turn ${a.turn}.${a.callIndex}: ${a.toolId}@${a.toolVersion}`);
}
```

**CLI** — `kx agent run`:

```bash
kx agent run --goal "Use the echo tool to repeat 'pong'." --max-tool-calls 20 --json
# { "answer": "...", "actions": [{ "tool_id": "mcp-echo/echo", "tool_version": "1", "turn": 1, "call_index": 0 }],
#   "run_handle": "<hex>", "instance_id": "<hex>" }
# exit 0 = answered · 1 = the run failed · 3 = timed out (resume with `kx react list`)
```

- **`context`** attaches published [context bundles](./context.md) the server resolves
  and injects (identity-bearing — a different context is a different run).
- **`inputs`** (`k=v` on the CLI, a map in the SDKs) fold into the goal prompt.
- **`max_tool_calls`** bounds the chain's *total* tool calls (default **20**, ceiling
  20). A single turn can fire **several tools at once** (the model emits N calls in one
  response — see [parallel tool calls](./tools.md#parallel-tool-calls-multi-element-tool-calling)),
  so this is independent of the model-turn budget (`max_turns`, default 8). The
  resolved server defaults show in **Settings → Workspace** and `kx info`.
- The returned **`AgentResult`** carries `answer` (+ `answer_bytes`), `actions` (the
  audited tool set), and the re-attachable `run_handle` / `instance_id`. A turn that
  fires N parallel tools yields **N actions** sharing `turn`, ordered by `call_index`.
  With `wait=False` you get the run handle back instead and assemble the result later.

> **Composing in a chain.** The agent-runner is the *steered* whole-run entry (the
> model picks tools turn by turn). To put a tool-using agent *step* inside a larger
> DAG, use the deterministic lane — `flow().agent(prompt, tools=[…])` or the
> `model@tool` chain step (see the [DSL reference](./chains/dsl-reference.md)) — where
> the granted tool set is fixed and part of the step's identity. There is no separate
> chains `agent()` node by design (it would be a second, divergent wire shape).

## Chat with tools

Attach an **explicit tool set** to a single chat turn and it becomes a bounded agentic
turn — the model may reason, call **only the tools you named**, observe, and answer. The
server builds the per-turn warrant **from the tools you passed** and re-verifies each at
every fire (SN-8); it is never a blanket auto-grant, and a tool you did not name cannot
fire. It is the one-liner entry to the same loop `flow().agent(prompt, tools=[…])` authors
as a chain step — the granted set is fixed and part of the turn's identity.

**Python** — `chat(prompt, *, tools=…, max_turns=…, max_tool_calls=…)`:

```python
from kortecx import KxClient

client = KxClient("http://localhost:50150", token="…")
answer = client.chat(
    "Use the echo tool to echo 'pong', then answer with it.",
    tools=["mcp-echo/echo@1"],          # or a bare "mcp-echo/echo" (version 1)
)
print(answer)                           # the settled answer text
# async: await AsyncKxClient(...).chat(prompt, tools=["mcp-echo/echo@1"])
```

**TypeScript** — `chat(prompt, { tools, maxTurns?, maxToolCalls? })`:

```ts
import { KxClient } from "@kortecx/sdk";

const client = new KxClient("http://localhost:50150", { token: "…" });
const answer = await client.chat(
  "Use the echo tool to echo 'pong', then answer with it.",
  { tools: ["mcp-echo/echo@1"] },
);
console.log(answer);
```

**CLI** — `kx chat --tools <id@ver,…>`:

```bash
kx chat "Use the echo tool to echo 'pong', then answer with it." \
  --tools mcp-echo/echo@1 --max-turns 8 --max-tool-calls 20
```

- **`tools`** is the exact granted set (`id@version`; comma-separated on the CLI, a list
  or a `{name: version}` map in the SDKs — a bare `id` defaults to version `1`). The turn
  lowers to one agentic MODEL step whose `tool_contract` is those tools — identical to
  `flow().agent(prompt, tools=[…])` — so the server builds the scoped warrant from it.
- **`max_turns`** (default 8) / **`max_tool_calls`** (default 20) bound the loop.
- Attaching tools does **not** compose with `--dataset` / `--image` (or `dataset=` /
  `image=`) yet — run them separately (a clear usage error, never a silent drop).

> **`chat(tools=…)` vs `run_agent`.** Both run the bounded loop; the difference is *who
> fixes the tool set*. `run_agent` uses the server's pre-wired react recipe (its warrant
> is fixed at provision). `chat(tools=…)` / `kx chat --tools` lets **you** name the exact
> tools for this turn — a scoped, per-turn grant — reusing the `flow().agent(tools=…)`
> lowering. (Attaching tools from the **console** chat composer is coming in a follow-up.)

## Searching a dataset (agentic RAG)

Pass `--dataset <name>` and the agent gets a read-only **`retrieve` tool** — it decides
when to search, phrases its own query, reads the passages, re-queries, and answers grounded
in what it found (the `kx/recipes/react-rag` recipe over the hybrid index). See
**[Agentic RAG](./agentic-rag.md)** for the full cross-surface walkthrough.

```bash
kx agent run --goal "What does the handbook say about parental leave?" --dataset handbook
```

## Vision in agents (agentic vision)

Attach an **image** and the agent reasons over it on **every turn** of the loop — not
just a one-shot caption. The image is carried **durably** through the chain (anchored on
the run's first ReAct turn and re-derived for every successor turn), so it survives
re-plan and a crash-and-recover exactly like the rest of the run. Single entry point, one
extra option — `image=` mirrors `client.chat(image=…)`:

**Python** — `run_agent(goal, image=…)` / `Agent.run(task, image=…)`:

```python
import kortecx as kx

img = open("chart.png", "rb").read()
result = kx.run_agent("Inspect this chart and use the echo tool to report the peak.", image=img)
print(result.answer)
# Agent class: kx.Agent("You are a data analyst.").run("What's the trend?", image=img)
```

**TypeScript** — `runAgent({ goal, image })` / `agent.run(task, { image })`:

```ts
import { runAgent } from "@kortecx/sdk";

const result = await runAgent({ goal: "Inspect this chart and report the peak.", image: bytes });
```

**CLI** — `kx agent run --image <path>`:

```bash
kx agent run --goal "Inspect this chart and use the echo tool to report the peak." --image ./chart.png
```

**In a chain** — the first-class `flow().image(ref)` node grounds the **next** agent step
(per-step: a later `.image()` before another `.agent()` grounds that step with a different
image). Upload the bytes once for a content ref, then chain:

```python
import kortecx as kx

kx_ = kx.default_client()
ref = kx_.put_content(open("invoice.png", "rb").read()).content_ref
out = (kx.flow()
       .image(ref).agent("Extract the line items from this invoice.", tools=["echo"])
       .then("Total them and flag any anomaly.")
       .run())
```
```ts
const ref = (await kx.putContent(bytes)).contentRef;
const out = await flow().image(ref).agent("Extract the line items.").then("Total them.").run({ client: kx });
```

Works on **both inference engines** (Ollama vision tags · llama.cpp + mmproj). When no
vision model is served the SDKs/CLI **fail closed** with a clear error (the image is never
silently dropped); in the console, agent mode **honest-degrades** to the text-only loop and
the attachment stays display-only. `dataset` + `image` together (vision-RAG) is a
follow-up. See [local inference engines](./local-inference-engines.md) to serve a vision
model and [chat](./chat.md#vision--ocr-attach-an-image) for single-shot image→text / OCR.

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

### Settling a tool-looping model

Some models keep calling tools without ever giving a final answer. On the **last
useful turn** — the one round before another tool call would exhaust the budget —
the runtime **nudges** the model: it appends a fixed steer telling it to stop
calling tools and answer directly from the observations it already has. Keep
`max_tool_calls < max_turns` (the default is `6 < 8`, and the runtime enforces it)
so there is always a turn left to settle on.

If the model ignores the nudge and still never answers, the chain **dead-letters
honestly** rather than silently quiescing: `kx agent run` exits **1** (a real
failure: "exhausted its tool-call budget without settling on an answer"), not
**3** (the resumable-timeout code). The Python and TypeScript SDKs raise
`KxRunFailed` (not `KxWaitTimeout`); the console's agent-loop strip flags it. This
graceful-recovery behavior is identical in the live server and in the embeddable
in-process loop (`kx-model-harness`), so an embedded agent recovers the same way.

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

## Self-checking with an LLM judge (opt-in)

Reasoning loops can be wrong. The **LLM-judge** is an *opt-in* verification gate
(T-AGENT2): the served model answers your prompt, then grades its own answer
against a rubric and commits a discrete **VALID** / **INVALID** verdict. It is a
durable, replayed fact — sampled once, committed, and reused on recovery (never
re-queried), so a run's outcome is stable.

Run the bundled `kx/recipes/judge` recipe (available when `kx serve` runs with a
model, `--features inference`):

```bash
kx invoke kx/recipes/judge --args '{"prompt":"What is the capital of France?"}' --wait
# state            COMMITTED
# verdict          valid
```

```python
import kortecx as kx
r = kx.invoke("kx/recipes/judge", {"prompt": "What is the capital of France?"}, wait=True)
print(r.verdict)        # "valid" | "invalid: judge: answer did not satisfy the rubric"
```

```typescript
const r = await kx.invoke(
  "kx/recipes/judge",
  { prompt: "What is the capital of France?" },
  { wait: true },
);
console.log(r.verdict); // "valid" | "invalid: …"
```

The graded answer itself is the producer step's committed output (visible in the
run's DAG and the mote inspector); the judge node carries the verdict.

**How it stays honest and reproducible:**

- **Server-derived authority (SN-8).** The judge runs under a server-built warrant
  — it dispatches the model but **cannot escalate authority** (no tools, no network
  unless explicitly granted). The model *proposes* a verdict; the runtime *parses*
  it to a discrete decision. There is **no similarity score** — only `VALID` /
  `INVALID`. An unparseable or ambiguous judge response **fails closed to invalid**
  (it never silently passes unverified output).
- **Opt-in, digest-scoped.** The default deterministic critics (schema / dedup /
  PII / stat-bounds) and the canonical demo are unchanged. A workflow that opts the
  judge **in** commits a `ReadOnlyNondet` verdict and so gets its *own* honest,
  replay-stable digest; a workflow that doesn't is byte-for-byte unaffected.
