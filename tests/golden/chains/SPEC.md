# Kortecx Chains DSL — grammar + canonical lowering (the cross-surface contract)

This is the **single source of truth** for the chain string-DSL. The Python, TypeScript,
and Rust (CLI) implementations MUST all parse + lower a chain expression to **byte-identical**
`(steps, edges)`, pinned by `corpus.json` in this directory (the GR12 tri-surface parity gate).

A chain expression composes **task handles** into a DAG. Each handle resolves (via a caller
`tasks` map) to a typed step (`pure` / `model` / `tool`; the palette grows per phase). The DSL
operators describe topology only — the server still compiles + warrants every step (SN-8); a
chain only changes what is PROPOSED.

### The `tool` step (PR-6b-2)

A `tool` handle fires a single REGISTERED tool as a standalone node. It carries a
`tool_contract = { tool_id: tool_version }` (the SERVER resolves the tool in its live registry and
builds the per-step warrant — the client never supplies a warrant or grants, SN-8) and lowers its
authored arguments to ONE **canonical-JSON object** under the reserved config key
`kx.tool.args` (`TOOL_ARGS_KEY`) in `params`. The canonical-JSON encoding is **keys sorted
ascending, compact separators (`,`/`:`), no floats** (the server schema is integer/bytes/bool/enum-typed) —
so `tool("web-search","1", q="kortecx", n=3)` lowers to `params["kx.tool.args"] = {"n":3,"q":"kortecx"}`
byte-identically across Python, TypeScript, and Rust. The coordinator re-derives + validates those
args against the tool's typed schema fail-closed at every lease (`resolve_authored_tool_args`).

## Grammar (EBNF)

```
chain    := orexpr
orexpr   := andexpr ( "|" andexpr )*     # parallel — LOOSEST
andexpr  := seqexpr ( "&" seqexpr )*     # parallel — tighter
seqexpr  := atom    ( ">" atom    )*     # sequential — tightest binary
atom     := handle | "[" chain "]"
handle   := [A-Za-z_][A-Za-z0-9_-]*
```

Whitespace between tokens is insignificant (`a>b` == `a > b`). Precedence, tightest → loosest:
`[ ]` grouping  >  `>` (sequential)  >  `&` (parallel)  >  `|` (parallel). All binary operators
are left-associative. This precedence **matches Python's native `>>` / `&` / `|`** so the string
DSL and the Python operator sugar (`a >> b`, `a | b`, `a & b`) lower identically.

`&` and `|` are the SAME operation (parallel merge — see below); they exist at two precedence
levels only to mirror Python and to let users express tight (`&`) vs loose (`|`) parallelism
without brackets. `[ ]` overrides precedence.

## Semantics — fragments

Every sub-expression evaluates to a **fragment** `{ entries, exits }` over the shared, ordered,
deduped node set (a handle that appears more than once is the SAME node — reuse builds DAGs):

- **atom `h`** (handle): register `h` in the node list on first appearance. Fragment
  `{ entries: [h], exits: [h] }`.
- **atom `[ expr ]`**: the fragment of `expr` unchanged (brackets are precedence-only).
- **`A > B`** (sequential, left-folded): add a DATA edge `(x, y)` for every `x ∈ A.exits` and
  `y ∈ B.entries`. Fragment `{ entries: A.entries, exits: B.exits }`.
- **`A & B`** / **`A | B`** (parallel merge, left-folded): add no edges. Fragment
  `{ entries: A.entries ++ B.entries, exits: A.exits ++ B.exits }` (order-preserving dedup).

So `a > [b & c]` fans OUT (`a→b`, `a→c`); `[a & b] > c` fans IN (`a→c`, `b→c`);
`[a & b] > [c & d]` is the full bipartite join.

## Canonical lowering (deterministic)

1. **Nodes**: in **first-appearance order** (the order each handle is first parsed as an atom,
   left-to-right). Node index = position in this list.
2. **Steps**: for each node in order, its `tasks[handle]` → `StepInput` verbatim.
3. **Edges**: the accumulated edge set, **deduped**, then **sorted ascending by
   `(parent_index, child_index)`**. Every edge is `edge = "data"`.
4. **seed**: the chain's seed (default `0`). **mode**: `"frozen"`.

The result feeds `BlueprintBuilder.add_step` / `add_edge` (one canonical lowering) →
`SubmitWorkflowRequest`.

## Validation (errors, fail-closed)

- **Empty expression** or **empty group `[]`** → parse error.
- **Unknown handle**: a parsed handle absent from `tasks` → error (`unknown task handle '<h>'`).
  Tasks defined but unused are ignored (lenient).
- **Cycle / self-loop** (`a > a`, `a > b | b > a`): reject with a cycle error (a Kahn topo check
  client-side; the server compile is the backstop). The DSL CAN express cycles via handle reuse,
  so this check is required.

## The corpus

`corpus.json` is an array of cases. A success case:

```json
{ "name": "...", "dsl": "a > [b & c]", "seed": 0,
  "tasks": { "a": {"kind":"pure"}, "b": {"kind":"pure"}, "c": {"kind":"pure"} },
  "expect": {
    "steps": [ {"kind":"pure","model_id":"","prompt":"","body_signature_id":null,"tool_contract":{},"params":{}}, ... ],
    "edges": [ {"parent":0,"child":1,"edge":"data"}, {"parent":0,"child":2,"edge":"data"} ] } }
```

A `tool` task spec carries `tool_contract` + (optional) structured `args`:
`{ "kind": "tool", "tool_contract": {"web-search":"1"}, "args": {"q":"kortecx","n":3} }`; its
`expect` step has the `tool_contract` and `params["kx.tool.args"]` = the canonical-JSON string. Each
surface lowers the structured `args` (Python/TS via the `tool()` factory, Rust via the CLI
`StepSpec.args`) and the corpus asserts the byte-identical canonical JSON.

`steps` are in node order; `params` values are strings (the pre-encoding lowering form — each SDK
UTF-8-encodes at `build()` time). An error case carries `"error": "<class>"` instead of `expect`,
where class ∈ `{parse, unknown_handle, cycle}`. Each implementation's test parses every case, and
for success cases asserts its lowered `(steps, edges)` deep-equals `expect`; for error cases
asserts the matching error class is raised.
