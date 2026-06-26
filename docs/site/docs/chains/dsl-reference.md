---
id: dsl-reference
title: Chains DSL reference
sidebar_label: DSL reference
description: The Kortecx chain operator grammar, precedence, canonical lowering, and worked examples.
---

# Chains DSL reference

A **chain** composes **task handles** into a DAG. Each handle resolves — via a
caller-supplied `tasks` map — to a typed step (`pure` or `model` today; the
palette grows per phase). The operators describe **topology only**: the server
still compiles and warrants every step (see
[Security](../security.md#model-proposes-runtime-enforces)). A chain only changes
what is *proposed*.

This page is distilled from the cross-surface contract. The Python, TypeScript,
and Rust (CLI) implementations all parse and lower a chain expression to
**byte-identical** `(steps, edges)`, pinned by a shared golden corpus.

## Grammar

```ebnf
chain    := orexpr
orexpr   := andexpr ( "|" andexpr )*     # parallel — LOOSEST
andexpr  := seqexpr ( "&" seqexpr )*     # parallel — tighter
seqexpr  := atom    ( ">" atom    )*     # sequential — tightest binary
atom     := handle grants? | "[" chain "]"
grants   := ( "@" handle )+              # tool tags on a MODEL handle (PR-9b)
handle   := [A-Za-z_][A-Za-z0-9_-]*
```

Whitespace between tokens is insignificant (`a>b` is the same as `a > b`).

## Precedence

Tightest → loosest:

| Operator | Meaning | Precedence | Associativity |
|---|---|---|---|
| `@` | tag a tool onto a MODEL handle (PR-9b) | tightest (handle suffix) | — |
| `[ ]` | grouping (overrides precedence) | tightest | — |
| `>` | sequential (add data edges) | tighter | left |
| `&` | parallel merge | looser | left |
| <code>&#124;</code> | parallel merge | loosest | left |

This precedence **matches Python's native `>>` / `&` / `|`**, so the string DSL
and the Python operator sugar (`a >> b`, `a & b`, `a | b`) lower identically.

:::info `&` and `|` are the same operation
Both are a **parallel merge** (they add no edges). They exist at two precedence
levels only to mirror Python and to let you express tight (`&`) versus loose
(`|`) grouping without brackets. `[ … ]` overrides precedence explicitly.
:::

## Semantics — fragments

Every sub-expression evaluates to a **fragment** `{ entries, exits }` over a
shared, ordered, deduped node set. A handle that appears more than once is the
**same node** — reuse is how you build DAGs.

| Expression | Edges added | Resulting fragment |
|---|---|---|
| `h` (handle) | none; registers `h` on first appearance | `{ entries: [h], exits: [h] }` |
| `[ expr ]` | the fragment of `expr`, unchanged | (brackets are precedence-only) |
| `A > B` | a **data** edge `(x, y)` for every `x ∈ A.exits`, `y ∈ B.entries` | `{ entries: A.entries, exits: B.exits }` |
| `A & B` / `A | B` | none | `{ entries: A.entries ++ B.entries, exits: A.exits ++ B.exits }` (order-preserving dedup) |

So:

- `a > [b & c]` **fans out** — `a→b`, `a→c`.
- `[a & b] > c` **fans in** — `a→c`, `b→c`.
- `[a & b] > [c & d]` is the full **bipartite join**.

## The deterministic-agentic step — `model@tool` (PR-9b)

A **MODEL** handle can tag tools with `@` to become a **deterministic-agentic
step** — a frozen-DAG model step that runs a *bounded* reason→tool→observe loop
over a fixed, author-declared, server-vetted tool-grant SET:

```
plan@web-search@fs-list > review
```

Here `plan` is one node (one DAG vertex) granted `{web-search, fs-list}`; `review`
is a downstream pure/model step. This is the **authored/deterministic** lane — the
DAG topology and the tool set are fixed at authoring (distinct from the *steered,
non-deterministic* `react` recipe, where the model picks tools dynamically).

- `@` binds **tighter than every operator** (it is a handle suffix); whitespace
  around it is insignificant (`p @ echo` == `p@echo`).
- Each `@tag` is a tool **name** (version defaults to `"1"`); tags are
  **order-preserving and deduped** (`p@x@x` == `p@x`). They lower into the model
  step's `tool_contract` (the same field a standalone `tool()` step uses). The
  **server** resolves each tagged tool in its live registry and builds the per-step
  warrant — you never supply a warrant or grants (SN-8).
- The bounded-loop **budget** (`max_turns` / `max_tool_calls`) rides the task spec,
  not the `@` grammar; absent ⇒ the server default (8 turns / 6 tool calls).
- `@` on a **non-model** handle (`pure@tool`) is a fail-closed authoring error.

:::info Authoring now, execution in PR-9b-2
The cross-surface **authoring** of `model@tool` steps (this grammar, the SDK
`tools=` factory, the golden corpus, the server-vetted per-step warrant) ships in
**PR-9b-1**. The bounded reason→tool→observe **loop execution** lands in **PR-9b-2**
— until then the server **fails closed** with a clear refusal when you submit one.
For tool-calling today, use a standalone `tool()` step or the `react` / `react-auto`
recipe.
:::

## Canonical lowering

A chain lowers deterministically to `(steps, edges)`:

1. **Nodes** — in **first-appearance order** (the order each handle is first
   parsed as an atom, left to right). The node index is its position in this
   list.
2. **Steps** — for each node in order, `tasks[handle]` becomes its `StepInput`
   verbatim.
3. **Edges** — the accumulated edge set, **deduped**, then **sorted ascending by
   `(parent_index, child_index)`**. Every edge is `edge = "data"`.
4. **seed** — the chain's seed (default `0`). **mode** — `"frozen"`.

The result feeds `BlueprintBuilder.add_step` / `add_edge` (one canonical
lowering) and produces a `SubmitWorkflowRequest`.

## Validation (fail-closed)

| Condition | Error class |
|---|---|
| Empty expression, or empty group `[]` | `parse` |
| A dangling/misplaced `@` (`p@`, `p@@x`, `@tool`) | `parse` |
| A parsed handle absent from `tasks` (`unknown task handle '<h>'`) | `unknown_handle` |
| A cycle or self-loop (`a > a`, `a > b | b > a`) | `cycle` |
| `@` tool grants on a non-model step (`pure@tool`) | `agentic_non_model` |

Tasks that are defined but never used are ignored (lenient). The DSL *can*
express cycles via handle reuse, so a client-side Kahn topological check rejects
them up front; the server compile is the backstop.

## Worked examples

These are drawn directly from the cross-surface golden corpus. Edges are
`(parent_index → child_index)` after canonical sort.

| DSL | Nodes (first-appearance order) | Edges | Shape |
|---|---|---|---|
| `a` | `a` | — | single step |
| `a > b` | `a, b` | `0→1` | sequential |
| `a > b > c` | `a, b, c` | `0→1`, `1→2` | pipeline |
| `a \| b` | `a, b` | — | two independent roots |
| `a & b` | `a, b` | — | two independent roots |
| `a > [b & c]` | `a, b, c` | `0→1`, `0→2` | **fan-out** |
| `[a & b] > c` | `a, b, c` | `0→2`, `1→2` | **fan-in** |
| `[a & b] > [c & d]` | `a, b, c, d` | `0→2`, `0→3`, `1→2`, `1→3` | **bipartite join** |
| `a > b \| c` | `a, b, c` | `0→1` | seq binds tighter than `\|` |
| `a > b & c` | `a, b, c` | `0→1` | seq binds tighter than `&` |
| `a & b > c` | `a, b, c` | `1→2` | seq binds tighter than `&` |
| `a \| b & c` | `a, b, c` | — | both are parallel merge |
| `[a > [b & c]] > d` | `a, b, c, d` | `0→1`, `0→2`, `1→3`, `2→3` | nested fan-out then fan-in |
| `a > b \| a > c` | `a, b, c` | `0→1`, `0→2` | reuse of `a` builds a fan-out |

### Precedence in practice

`a > b | c` and `a > b & c` both parse as `(a > b) | c` and `(a > b) & c` because
`>` is tighter than `&`/`|`. Only `a > b` adds an edge (`0→1`); `c` is an
independent root. To fan `a` out to both `b` and `c`, group the parallel side:
`a > [b & c]`.

### A model step in a chain

:::note Model-step execution (in progress)
The DSL **lowers** `model` steps correctly today (shown above + corpus-pinned). **Live model execution inside an authored blueprint** lands with the `SubmitWorkflow` model-route fix (a near-term follow-up): the server currently builds a blueprint model step's warrant with a placeholder model route, so a model step against a non-default served model dead-letters fail-closed. `pure` chains run end-to-end now; published model **recipes** (e.g. `kx/recipes/chat`) run via `invoke` today.
:::

A `tasks` entry can be a `model` step. For example, the chain `gen > sum` with:

```json
{
  "gen": { "kind": "model", "model_id": "kx-serve:qwen3-4b-q4_k_m", "prompt": "Summarize the input." },
  "sum": { "kind": "pure", "params": { "label": "final" } }
}
```

lowers to two steps (`gen` then `sum`) with a single data edge `0→1`. The model
step's `model_id` and `prompt` are carried into its `StepInput`; the pure step
carries its `params`. Params values are strings in the lowering form — each SDK
UTF-8-encodes them at `build()` time.

### Fewer inputs from the JSON / CLI surface

The author-side step is **terse**: the runtime infers what it can, and you only
spell out what you actually mean.

- **`kind` is optional.** Omit it and the CLI infers it from the fields present — a
  `model_id`/`prompt` ⇒ `model`, a `tool_contract` (with no model fields) ⇒ `tool`,
  anything else ⇒ `pure`. An explicit `kind` is an override that must *agree* with the
  fields (a `kind:"pure"` next to a `model_id` is a fail-closed error, not a silent
  drop). A `model` step that also carries a `tool_contract` stays a `model` step (the
  deterministic-agentic step).
- **`model_id` is optional.** Omit it and the server binds the served model; set a
  client `default_model` (Python `KxClient(default_model=…)` / TS `{ defaultModel }` /
  the `KX_DEFAULT_MODEL` env var) to fill it for every blank MODEL step at submit.
- **`reasoning`** (`full` / `minimal` / `off` / `strip`) is a typed knob on the SDK
  `model()` factory — absent leaves the model's own behavior (and the step identity)
  unchanged.

```bash
# A whole chain authored inline — no tasks file, no `kind`:
kx chain run "plan > review" \
  --task plan='{"prompt":"Research the topic."}' \
  --task review='{}' --wait
```

`--task name='{…}'` (repeatable) and `--tasks-json '{…}'` are inline alternatives to
`--tasks <file>`; all three merge into one handle → step map (fail-closed on a handle
defined twice).

### The authoring ladder

Reach for the lowest rung that fits — each lowers to the same `SubmitWorkflow` the
server compiles and warrants (SN-8):

1. **`kx invoke <recipe>`** — run a published recipe by handle (`--args '{…}'`). The
   easiest front door for a ready-made capability.
2. **The chain DSL** — `chain("a > [b & c]", …)` / `a >> (b & c)` — compose your own
   handles into a DAG. This page.
3. **A raw blueprint** — `kx blueprint run --file dag.json` — full control over every
   step and edge.

### Authored vs steered tool-calling

There are two lanes for tool-calling, and the API names mirror the distinction:

- **Authored / deterministic** — the `model@tool` step (above): a *fixed*, server-vetted
  tool-grant SET on a frozen-DAG step. Replayable; the tool set is part of the step's
  identity. (Execution lands in PR-9b-2; see the note above.)
- **Steered / dynamic** — the `kx/recipes/react` recipe: the model picks tools turn by
  turn from the granted set. Use this for open-ended agents today.

### Attaching context bundles

A chain can attach [context bundles](../context.md) — named, content-addressed
grounding the model reasons over. Context is **chain-level, not a node**: the
server injects it into the chain's entry step(s), so position is irrelevant.
Attach handles via the `context` option (or the fluent `.context(...)`, or the CLI
`--context` flag, repeatable):

```python
chain("plan > write", tasks=tasks, context=["team/ctx/spec"])
```

```ts
chain("plan > write", { tasks, context: ["team/ctx/spec"] });
```

```bash
kx chain run "plan > write" --tasks tasks.json --context team/ctx/spec
```

### Attaching an image (agentic vision)

The fluent `.image(ref)` node grounds the **next** agent step with an image the served
vision model reasons over on **every turn** of that step's loop (carried durably across
the chain). It is **per-step**: a later `.image()` before another `.agent()` grounds that
step with a different image. `ref` is a 64-hex content ref — upload the bytes once
(`client.put_content(data).content_ref`), then chain (the chain lowers client-free, so the
ref is the portable handle):

```python
out = (kx.flow()
       .image(ref).agent("Read the line items from this invoice.", tools=["echo"])
       .then("Total them and flag anomalies.")
       .run())
```

```ts
await flow().image(ref).agent("Read the line items.").then("Total them.").run({ client: kx });
```

See [Agents & reasoning → Vision in agents](../agent-runner.md#vision-in-agents-agentic-vision)
for `run_agent(image=…)` / `Agent.run(task, image=…)` / `kx agent run --image`.

The handles lower **verbatim** into the request's `context_bundles` (no DSL-side
sort or dedup — the server canonicalizes the sorted ref-set at bind, SN-8). A
chain with no attached context lowers byte-identically to pre-context-bundle, and
the attachment's byte-identity across Python, TypeScript, and the CLI is pinned by
the golden corpus alongside the topology.

## Portable blueprints (export / import)

A lowered chain can be saved as a **portable blueprint JSON** — the exact shape
`kx blueprint run --file` consumes — and re-run anywhere:

```bash
kx chain run "a > b" --tasks tasks.json --emit-blueprint plan.json --dry-run
kx blueprint run    --file plan.json --wait     # run it
kx blueprint import --file plan.json            # validate + summarize offline
```

The SDKs mirror this: `flow.export(path)` / `Chain.to_blueprint()` and
`Chain.from_blueprint(_file)`. Export → import re-compiles to a **byte-identical**
request. The artifact pins explicit `kind` but leaves `model_id` as authored (empty binds
the serve's model, SN-8), so a blueprint is portable across serves. See
[Blueprint builder → Portable blueprints](../blueprint-builder.md#portable-blueprints--export--import).

## Per-language authoring

- **[Chains in Python](./python.md)** — the `chain()` string DSL plus the
  `>>` / `&` / `|` operator sugar.
- **[Chains in TypeScript](./typescript.md)** — the `chain()` string DSL plus the
  combinator API.
