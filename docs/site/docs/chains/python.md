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

## The string DSL

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
