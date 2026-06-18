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
atom     := handle | "[" chain "]"
handle   := [A-Za-z_][A-Za-z0-9_-]*
```

Whitespace between tokens is insignificant (`a>b` is the same as `a > b`).

## Precedence

Tightest → loosest:

| Operator | Meaning | Precedence | Associativity |
|---|---|---|---|
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
| A parsed handle absent from `tasks` (`unknown task handle '<h>'`) | `unknown_handle` |
| A cycle or self-loop (`a > a`, `a > b | b > a`) | `cycle` |

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

The handles lower **verbatim** into the request's `context_bundles` (no DSL-side
sort or dedup — the server canonicalizes the sorted ref-set at bind, SN-8). A
chain with no attached context lowers byte-identically to pre-context-bundle, and
the attachment's byte-identity across Python, TypeScript, and the CLI is pinned by
the golden corpus alongside the topology.

## Per-language authoring

- **[Chains in Python](./python.md)** — the `chain()` string DSL plus the
  `>>` / `&` / `|` operator sugar.
- **[Chains in TypeScript](./typescript.md)** — the `chain()` string DSL plus the
  combinator API.
