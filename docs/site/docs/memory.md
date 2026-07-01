---
id: memory
title: Durable Memory
sidebar_label: Durable Memory
description: Give a local agent durable, cross-run memory — remember a fact in one run, recall it by meaning in the next. The remember/recall tools + the react-memory recipe, reachable from the one entry point (kx / KxClient), with provable, replayable recall.
---

# Durable Memory

An agent that forgets everything between runs cannot get better at your work. **Durable
memory** lets a local agent **remember** a fact in one run and **recall** it — by meaning —
in a later one. It is cross-run, scoped to you, forgettable, and **provable**: every recall
commits the **ordered content-refs** of the memories it read (scores stay out — the model
proposes, the runtime records exact references only), so you can prove exactly what the agent
recalled at each step and recover a crashed agent to that same recall state.

Memory mirrors the [RAG](./datasets.md) substrate, so it inherits the runtime's guarantees:
the recall result is a committed `ReadOnlyNondet` fact; the store (`memory.db`) is an
off-digest, rebuildable sidecar (lose it and it rebuilds from its durable rows) — **no journal
schema change**. It is reachable from the **one entry point** — `kx` / `KxClient` — exactly
like every other capability.

## The two tools

The `kx/recipes/react-memory` recipe (a sibling of [`react`](./agent-runner.md) and
`react-rag`) grants the agent two read/write built-in tools:

```json
remember@1 — {"content": <fact to remember>, "kind": "semantic"|"episodic" (optional)}
           → { "memory_id": <hash>, "stored": true }
recall@1   — {"query": <what to recall>, "k": <1..64, optional>}
           → ordered memories: [{ "ref": <hash>, "text": ... }]
```

Both have **no egress and no filesystem scope** (the store is reached in-process). `recall` is
**read-only** (`Readback` — the human-in-the-loop gate auto-proceeds it). `remember` is an
**idempotent write** (`Token`): remembering the same fact twice is a durable no-op, so a
crash-recovery re-dispatch never duplicates a memory, and it **auto-proceeds** too (a local,
reversible, no-egress write). Both **fail soft** — a missing embedder or an empty store returns
an honest empty observation the agent reads and recovers from; the loop never dead-letters.

Every memory is **scoped to your own principal** (server-derived) — a client can never reach
another principal's memories.

## Enable it

Memory is off by default (it is a new per-principal store). Turn it on with a served model
(for embedding), the `hnsw` index, and the `KX_SERVE_MEMORY` flag:

```bash
KX_SERVE_MEMORY=1 kx serve --features inference,hnsw --model <a-gemma-model>
```

Without it, the memory commands + RPCs answer `Unimplemented` honestly.

## Remember & recall directly

**CLI** — `kx memory`:

```bash
kx memory add "the project deadline is March 3rd"
kx memory add "the client prefers email over calls" --kind episodic
kx memory recall --text "when is my deadline?"      # → the deadline fact, by meaning
kx memory list                                       # the episodic log, newest-first
kx memory forget <memory_id>                         # erase one by its content id
```

**Python** — `kx.memory`:

```python
from kortecx import KxClient

kx = KxClient("http://127.0.0.1:50151")
kx.memory.store("the project deadline is March 3rd")
hits = kx.memory.recall("when is my deadline?", k=5)
print(hits[0].text)                # "the project deadline is March 3rd"
for m in kx.memory.list():         # the episodic log
    print(m.kind, m.text)
```

**TypeScript** — `client.memory`:

```ts
import { KxClient } from "@kortecx/sdk/node";

const kx = new KxClient("http://127.0.0.1:50151");
await kx.memory.store("the project deadline is March 3rd");
const hits = await kx.memory.recall("when is my deadline?", { k: 5 });
console.log(hits[0].text);
```

## Chain memory into an agent

The headline path: **seed** facts and let a `react-memory` agent recall them autonomously —
memory is chained from the same fluent entry point as everything else.

**Python** — `flow().with_memory(...)`:

```python
answer = (
    kx.flow()
      .with_memory([
          "the project deadline is March 3rd",
          "the client prefers email over calls",
      ])
      .agent("How should I follow up, and by when?",
             recipe=REACT_MEMORY_RECIPE_HANDLE)
      .run()
)
```

**TypeScript** — `flow().withMemory(...)`:

```ts
const answer = await kx
  .flow()
  .withMemory(["the project deadline is March 3rd", "the client prefers email over calls"])
  .agent("How should I follow up, and by when?", { recipe: REACT_MEMORY_RECIPE_HANDLE })
  .run();
```

`with_memory` / `withMemory` is pure pre-submit sugar over `store` — it never changes the
lowered workflow (the golden digest holds); the store is an imperative side effect, not a DAG
node.

## In the console

The **Context → Memories** tab is the memory workbench: remember a fact, recall by meaning
(each hit shows its DISPLAY-only similarity score), and browse the per-principal episodic log
with per-item forget. Without memory enabled it degrades to an honest not-wired state.

## Operator knobs

| Env var | Default | Effect |
|---|---|---|
| `KX_SERVE_MEMORY` | off | Enable the durable-memory subsystem (the `remember`/`recall` tools, the `react-memory` recipe, and the memory RPCs). |
| `KX_SERVE_EMBED_MODEL` | (the primary served model) | The embed model for storing/recalling text; set it to a dedicated embedder (e.g. `embeddinggemma`) for sharper recall. |

## What is Cloud

Cross-run memory for **one principal on one node** is OSS. Sharing memory across parties,
tenants, or nodes, plus consolidation/decay policy tuning at scale, are part of Kortecx Cloud.
Consolidation (summarizing episodes into semantic facts) and decay land in a later release.
