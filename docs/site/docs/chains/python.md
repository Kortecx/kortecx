---
id: python
title: Chains in Python
sidebar_label: Python
description: Author Kortecx chains in Python — the string DSL and the operator/combinator API.
---

# Chains in Python

The Python SDK (`pip install kortecx`) authors chains two equivalent ways: a
**string DSL** and **operator sugar** (`>>` / `&` / `|`). Both lower to the same
canonical `(steps, edges)` as the CLI and the TypeScript SDK — see the
[DSL reference](./dsl-reference.md#canonical-lowering).

## Install

```bash
pip install kortecx            # core client (grpcio + protobuf)
pip install 'kortecx[ws]'      # + the optional WebSocket live-tail
```

## The fluent builder (recommended)

The friendliest way to author a chain reads top-to-bottom and fills in defaults — no
`tasks` map, no `kind`, no `model_id`:

```python
import kortecx as kx

out = (kx.flow()
       .agent("Research the topic", tools=["web-search"])   # an agent (MODEL) step
       .then("Critique the findings")                       # a follow-on agent
       .run())                                              # default client + wait
print(out.text)
```

`kx.flow()` builds the SAME `(steps, edges)` as the operator/DSL forms (it lowers
byte-identically — a Flow is sugar, never a new wire shape). Builders:
`.agent(prompt, tools=…, reasoning=…)` · `.step(**params)` (pure) · `.tool(id, ver, **args)` ·
`.then(item)` (sequential — a string is an agent) · `.parallel(*items)` (fan-out / fan-in) ·
`.context(*handles)`. Terminate with `.run()` (waits for the `Result`), `.submit()` (a
non-blocking `Run` handle), `.to_chain()` / `.build()` to inspect, or `.export(path)` to
save a **portable blueprint** (see [Export / import](#export--import-a-portable-blueprint)).

A `Run` (from `.submit()` or `.run(wait=False)`) drives the run without blocking:
`run.events()` (live projection deltas), `run.wait()` (the first committed Mote — the
await-any path), `run.tokens(mote)` (one model mote's advisory token chunks). The
`Result` exposes `.text` / `.bytes` and `.json()` (the `kx … --wait --json` shape).

### Export / import a portable blueprint

`.export(path)` writes the lowered chain as a portable blueprint JSON (the exact
`kx blueprint run --file` input — save / version / share / re-run it); `to_blueprint()`
returns the dict. `Chain.from_blueprint(spec)` / `Chain.from_blueprint_file(path)`
compile one back into a `SubmitWorkflowRequest`:

```python
import kortecx as kx

kx.flow().agent("Research the topic").then("Critique it").export("plan.json")
req = kx.Chain.from_blueprint_file("plan.json")   # → a SubmitWorkflowRequest
client.submit_workflow(req, wait=True)
```

The artifact is self-describing (explicit `kind`) and portable — `model_id` stays as
authored (empty binds the serve's model at submit, SN-8). Export → import re-compiles to
the IDENTICAL request as `.build()`. See [Blueprint builder → Portable blueprints](../blueprint-builder.md#portable-blueprints--export--import).

### A reusable Agent

```python
import kortecx as kx

analyst = kx.Agent("You are a research analyst.", tools=["web-search", "fs-list"])
print(analyst.run("Summarize the kortecx README").text)
```

`kx.Agent` carries instructions + an optional tool set + model/loop config. The **default
lane is deterministic/frozen** — a single agent step with a FIXED tool-grant SET
(replayable; the tools are part of the step's identity; execution lands with PR-9b-2).
`dynamic=True` routes to the **steered** `kx/recipes/react` recipe, where the model picks
tools turn by turn (works today). `analyst.stream(task)` submits without blocking and
returns a `Run` (consume `.events()` / `.tokens(mote)`).

The tool set may include your own functions — decorate one with `@kx.tool` and pass it in
`tools=[...]`; the SDK registers it as a local stdio MCP server the runtime dials. See
[Local function tools](../tools.md#local-function-tools-kxtool--localtool).

### Zero-config

`import kortecx as kx; kx.run(...)` uses a lazily-built default client. Config order:
explicit → env (`KX_ENDPOINT` / `KX_TOKEN` / `KX_DEFAULT_MODEL`) → `~/.kortecx/config.toml`
→ the loopback default. Pass an explicit `KxClient` (the `client=` kwarg, or
`kx.set_default_client(...)`) for full control — the singleton is a script convenience
(construct explicit clients for concurrent/async work).

```python
kx.run("Summarize this design doc.", reasoning="minimal")   # a one-line agent
```

## The string DSL

The operator sugar (`a >> b`), the string DSL, and the raw blueprint JSON remain as
power forms — all lower identically to the fluent builder above.

Compose published task handles into a DAG with a single expression. The `tasks`
map resolves each handle to a typed step.

```python
from kortecx import KxClient, chain

tasks = {
    "a": {"kind": "pure"},
    "b": {"kind": "pure"},
    "c": {"kind": "pure"},
}

# Fan-out: `a` feeds both `b` and `c`  →  edges 0→1, 0→2
spec = chain("a > [b & c]", tasks=tasks, seed=0)

with KxClient("http://127.0.0.1:50151") as kx:
    result = kx.submit_workflow(spec, wait=True)
    print(result.instance_id)
```

The expression `"a > [b & c]"` lowers to nodes `a, b, c` (first-appearance order)
with data edges `0→1` and `0→2`, byte-identical to every other surface.

## Operator sugar

The DSL precedence **matches Python's native operators**, so `>>` / `&` / `|`
lower identically to the string form:

```python
# These two are equivalent:
chain("a > [b & c]", tasks=tasks)
a >> (b & c)                        # operator sugar — same lowering
```

| Python operator | String DSL | Meaning |
|---|---|---|
| `a >> b` | `a > b` | sequential — add a data edge |
| `a & b` | `a & b` | parallel merge (tighter) |
| `a \| b` | `a \| b` | parallel merge (looser) |
| `(…)` | `[ … ]` | grouping |

Because `>>` binds tighter than `&`, which binds tighter than `|` — exactly as in
the [precedence table](./dsl-reference.md#precedence) — `a >> b | c` groups as
`(a >> b) | c`, matching the string DSL.

## Deterministic-agentic step — `model@tool` (PR-9b)

Tag tools onto a MODEL step to make it a
[deterministic-agentic step](./dsl-reference.md#the-deterministic-agentic-step--modeltool-pr-9b)
— a bounded reason→tool→observe loop over a server-vetted tool-grant SET. The
string DSL `@` grammar and the `model(tools=...)` factory lower **identically**:

```python
from kortecx import chain
from kortecx.chains import model, pure

# String DSL: `plan` is granted {web-search, fs-list}; `review` is downstream.
spec = chain(
    "plan@web-search@fs-list > review",
    tasks={
        "plan": model("kx-serve:my-model", "Research the topic.", max_turns=4, max_tool_calls=3),
        "review": pure(),
    },
)

# The factory form lowers to the same (steps, edges):
plan = model("kx-serve:my-model", "Research the topic.",
             tools=["web-search", "fs-list"], max_turns=4, max_tool_calls=3)
```

`tools=` accepts a list of names (version `"1"`) or a `{name: version}` map; the
budget (`max_turns` / `max_tool_calls`) defaults to 8 / 6 when omitted. The server
vets every tagged tool and builds the per-step warrant (SN-8).

:::info Authoring now, execution in PR-9b-2
Authoring is available across every surface in PR-9b-1; the bounded-loop
**execution** lands in PR-9b-2 — until then the server fails closed on a submitted
`model@tool` step. For tool-calling today, use a standalone `tool()` step or the
`react` / `react-auto` recipe.
:::

## A model step

:::note Model-step execution (in progress)
The DSL **lowers** `model` steps correctly today (shown above + corpus-pinned). **Live model execution inside an authored blueprint** lands with the `SubmitWorkflow` model-route fix (a near-term follow-up): the server currently builds a blueprint model step's warrant with a placeholder model route, so a model step against a non-default served model dead-letters fail-closed. `pure` chains run end-to-end now; published model **recipes** (e.g. `kx/recipes/chat`) run via `invoke` today.
:::

A `tasks` entry can be a `model` step carrying a `model_id` and `prompt`:

```python
tasks = {
    "gen": {"kind": "model", "model_id": "kx-serve:qwen3-4b-q4_k_m", "prompt": "Summarize the input."},
    "sum": {"kind": "pure", "params": {"label": "final"}},
}

spec = chain("gen > sum", tasks=tasks)   # two steps, edge 0→1
```

## Fewer inputs, sensible defaults

Authoring asks only for what is essential — the runtime infers the rest.

**Optional `model_id` + a client `default_model`.** Omit `model_id` and the server
binds the served model (SN-8). Set a `default_model` on the client (or the
`KX_DEFAULT_MODEL` env var) to fill it for every MODEL step that left it blank:

```python
from kortecx import KxClient
from kortecx.chains import model

with KxClient(default_model="kx-serve:qwen3-4b-q4_k_m") as kx:
    spec = chain("plan > review", tasks={
        "plan":   model(prompt="Research the topic."),   # no model_id
        "review": model(prompt="Critique the findings."),
    })
    print(kx.run_chain(spec, wait=True).text)
```

The default fills in at submit — the canonical lowering is untouched (it carries an
empty `model_id`, which the server binds to the served model).

**`reasoning=` — the typed knob.** Steer the model's native think mode with a typed
keyword instead of a raw param string. Omit it and the model's own behavior (and the
step's identity) is unchanged; set it for a new, reproducible step:

```python
model(prompt="Solve it carefully.", reasoning="full")     # "full" | "minimal" | "off" | "strip"
```

:::tip Author a quick chain from the CLI with no file
The `kx chain` CLI infers each step's `kind` (omit it — a `prompt` ⇒ a model step, a
`tool_contract` ⇒ a tool step, else a pure step) and takes tasks inline:

```bash
kx chain run "plan > review" \
  --task plan='{"prompt":"Research the topic."}' \
  --task review='{"prompt":"Critique the findings."}' --wait
```
:::

## Attaching context bundles

Ground the chain with [context bundles](../context.md) via the `context=` argument,
or the fluent `.context(...)` (which appends and returns a new chain). Context is
chain-level — the server injects it into the chain's entry step(s):

```python
chain("gen > sum", tasks=tasks, context=["team/ctx/spec"])
chain("gen > sum", tasks=tasks).context("team/ctx/spec", "team/ctx/notes")
```

Handles ride verbatim into the request (the server canonicalizes at bind). A chain
with no attached context is byte-identical to one authored before context bundles.

## Validation

The same fail-closed rules as the [DSL reference](./dsl-reference.md#validation-fail-closed)
apply, raised as typed Python errors:

```python
chain("a > a", tasks={"a": {"kind": "pure"}})   # raises — cycle
chain("a > z", tasks={"a": {"kind": "pure"}})   # raises — unknown task handle 'z'
chain("", tasks={"a": {"kind": "pure"}})        # raises — parse error
```

## Running it

Once built, a chain submits through the same path as any Blueprint:

```python
with KxClient("http://127.0.0.1:50151") as kx:
    result = kx.submit_workflow(spec, wait=True)
    print(result.text)
```

Every `instance_id` and `MoteId` is **server-derived** — the SDK carries the
server's bytes and never constructs an identity
([SN-8](../security.md#identity-is-server-derived)).

## See also

- [Chains DSL reference](./dsl-reference.md) — the full grammar and worked
  examples.
- [Chains in TypeScript](./typescript.md) — the same chains in TS.
- [`bindings/python/README.md`](https://github.com/Kortecx/kortecx/blob/main/bindings/python/README.md)
  — the full client SDK surface.
