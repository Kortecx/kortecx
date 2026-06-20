---
id: typescript
title: Chains in TypeScript
sidebar_label: TypeScript
description: Author Kortecx chains in TypeScript/JavaScript — the string DSL and the combinator API.
---

# Chains in TypeScript

The TypeScript SDK (`npm install @kortecx/sdk`) authors chains two equivalent
ways: a **string DSL** and a **combinator API**. Both lower to the same canonical
`(steps, edges)` as the CLI and the Python SDK — see the
[DSL reference](./dsl-reference.md#canonical-lowering).

## Install

```bash
npm install @kortecx/sdk       # node + browser entry points
npm install ws                 # optional: node live-tail
```

## The string DSL

Compose published task handles into a DAG with a single expression. The `tasks`
map resolves each handle to a typed step.

```ts
import { KxClient, chain } from "@kortecx/sdk";

const tasks = {
  a: { kind: "pure" },
  b: { kind: "pure" },
  c: { kind: "pure" },
};

// Fan-out: `a` feeds both `b` and `c`  →  edges 0→1, 0→2
const spec = chain("a > [b & c]", { tasks, seed: 0 });

const kx = new KxClient("http://127.0.0.1:50151");
const result = await kx.submitWorkflow(spec, { wait: true });
console.log(result.instanceId);
kx.close();
```

The expression `"a > [b & c]"` lowers to nodes `a, b, c` (first-appearance order)
with data edges `0→1` and `0→2`, byte-identical to every other surface.

## The combinator API

For programmatic composition, the combinators map one-to-one onto the operators:

```ts
import { seq, par, group } from "@kortecx/sdk";

// Equivalent to chain("a > [b & c]")
const spec = seq("a", group(par("b", "c")));
```

| Combinator | String DSL | Meaning |
|---|---|---|
| `seq(a, b)` | `a > b` | sequential — add a data edge |
| `par(a, b)` | `a & b` / `a \| b` | parallel merge |
| `group(expr)` | `[ … ]` | grouping |

Because the string DSL is the canonical contract, prefer `chain("…")` for
readability; reach for the combinators when you are assembling topology
dynamically.

## Deterministic-agentic step — `model@tool` (PR-9b)

Tag tools onto a MODEL step to make it a
[deterministic-agentic step](./dsl-reference.md#the-deterministic-agentic-step--modeltool-pr-9b)
— a bounded reason→tool→observe loop over a server-vetted tool-grant SET. The
string DSL `@` grammar and the `task.model(..., { tools })` factory lower
**identically**:

```ts
import { chain, task } from "@kortecx/sdk";

// String DSL: `plan` is granted {web-search, fs-list}; `review` is downstream.
const spec = chain("plan@web-search@fs-list > review", {
  tasks: {
    plan: task.model("kx-serve:my-model", "Research the topic.", {}, { maxTurns: 4, maxToolCalls: 3 }),
    review: task.pure(),
  },
});

// The factory form lowers to the same (steps, edges):
const plan = task.model("kx-serve:my-model", "Research the topic.", {}, {
  tools: ["web-search", "fs-list"],
  maxTurns: 4,
  maxToolCalls: 3,
});
```

`tools` accepts an array of names (version `"1"`) or a `{ name: version }` record;
the budget (`maxTurns` / `maxToolCalls`) defaults to 8 / 6 when omitted. The server
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

A `tasks` entry can be a `model` step carrying a `modelId` and `prompt`:

```ts
const tasks = {
  gen: { kind: "model", modelId: "kx-serve:qwen3-4b-q4_k_m", prompt: "Summarize the input." },
  sum: { kind: "pure", params: { label: "final" } },
};

const spec = chain("gen > sum", { tasks });   // two steps, edge 0→1
```

## Fewer inputs, sensible defaults

Authoring asks only for what is essential — the runtime infers the rest.

**Optional `modelId` + a client `defaultModel`.** Omit `modelId` and the server binds
the served model (SN-8). Set a `defaultModel` on the client (or, on Node, the
`KX_DEFAULT_MODEL` env var) to fill it for every MODEL step that left it blank:

```ts
import { KxClient, chain, task } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151", { defaultModel: "kx-serve:qwen3-4b-q4_k_m" });
const spec = chain("plan > review", {
  tasks: {
    plan: task.model("", "Research the topic."),     // no modelId
    review: task.model("", "Critique the findings."),
  },
});
console.log((await kx.runChain(spec, { wait: true })).text);
kx.close();
```

The default fills in at submit — the canonical lowering is untouched (it carries an
empty `modelId`, which the server binds to the served model).

**`reasoning` — the typed knob.** Steer the model's native think mode with a typed
option instead of a raw param string. Omit it and the model's own behavior (and the
step's identity) is unchanged; set it for a new, reproducible step:

```ts
task.model("", "Solve it carefully.", {}, { reasoning: "full" }); // "full"|"minimal"|"off"|"strip"
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

Ground the chain with [context bundles](../context.md) via the `context` option,
or the fluent `.context(...)` (which appends and returns a new chain). Context is
chain-level — the server injects it into the chain's entry step(s):

```ts
chain("gen > sum", { tasks, context: ["team/ctx/spec"] });
chain("gen > sum", { tasks }).context("team/ctx/spec", "team/ctx/notes");
chainFrom(seq(gen, sum), { context: ["team/ctx/spec"] });
```

Handles ride verbatim into the request (the server canonicalizes at bind). A chain
with no attached context is byte-identical to one authored before context bundles.

## Validation

The same fail-closed rules as the [DSL reference](./dsl-reference.md#validation-fail-closed)
apply, thrown as typed errors:

```ts
chain("a > a", { tasks: { a: { kind: "pure" } } });   // throws — cycle
chain("a > z", { tasks: { a: { kind: "pure" } } });   // throws — unknown task handle 'z'
chain("", { tasks: { a: { kind: "pure" } } });        // throws — parse error
```

## Running it

Once built, a chain submits through the same path as any Blueprint:

```ts
const kx = new KxClient("http://127.0.0.1:50151");
const result = await kx.submitWorkflow(spec, { wait: true });
console.log(result.state, result.instanceId, result.text);
kx.close();
```

Every `instanceId` and `MoteId` is **server-derived** — the SDK carries the
server's bytes and never constructs an identity
([SN-8](../security.md#identity-is-server-derived)).

## See also

- [Chains DSL reference](./dsl-reference.md) — the full grammar and worked
  examples.
- [Chains in Python](./python.md) — the same chains in Python.
- [`bindings/typescript/README.md`](https://github.com/Kortecx/kortecx/blob/main/bindings/typescript/README.md)
  — the full client SDK surface.
