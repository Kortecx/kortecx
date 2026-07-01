# LLM rerank (durable, replayable)

The **LLM listwise rerank** reorders a RAG retrieval's candidate passages by relevance
using the served model, so the most useful passage is read first. Unlike the always-on
[deterministic MMR rerank](./datasets.md#hybrid-retrieval--chunking), the LLM rerank is a
**durable, replayable coordinator turn**: the model proposes a permutation, the runtime
enforces it fail-closed, and the reorder is committed as an auditable `ReRankRound` fact
that survives a crash and re-serves from commit on replay (it is never re-sampled).

## Enable it

The LLM rerank is **off by default**. Turn it on for a `kx serve` process with:

```sh
KX_SERVE_RAG_LLM_RERANK=1 kx serve --features inference,hnsw
```

It then applies automatically on both live RAG paths:

- **Agentic `retrieve` loop (`react-rag`)** — after the agent's `retrieve@1` observation
  commits, its passages are reranked before the next reasoning turn reads them.
- **Grounded answer (`chat-rag` / `vision-rag`)** — the grounded passages are reranked
  before the single answer step dispatches (the answer is held until the rerank settles).

There is no client-chosen knob: the rerank runs model-side under the run's own warrant
(the model proposes an order; the runtime enforces exact validity — SN-8). The
deterministic MMR rerank remains the always-on default and is unaffected.

## Contract

- **Fail-closed.** Any non-permutation output, a dispatch error, or a shape mismatch keeps
  the upstream (RRF/MMR) order — a rerank can never reorder into garbage and never wedges
  an answerable RAG turn.
- **Off-budget.** The rerank does not consume the agent's ReAct `max_turns` /
  `max_tool_calls`; it is a separate `ReRankRound` fact.
- **Durable + replayable.** The reorder is a committed fact; recovery re-derives it from
  committed state, and replay serves the same order (recovery/audit/time-travel — not a
  re-run).
- **Dual-engine.** Ollama applies a strict whole-response JSON `format`; llama.cpp relies
  on the model plus the fail-closed parser (see [engine notes](./local-inference-engines.md)).

## Observe it — one entry point, chained everywhere

Every rerank is recorded as a `ReRankRound` fact you can list from the single `kx` / SDK
entry point:

```sh
# CLI
kx rerank list                       # recent rerank rounds (round · outcome · model · n · permutation)
kx rerank list --instance <hex>      # scoped to one run
kx rerank list --limit 20 --json     # machine-readable
```

```python
# Python SDK — the same client that submits runs lists reranks
from kortecx import KxClient
kx = KxClient("http://127.0.0.1:50051")
page = kx.list_rerank_turns(limit=20)          # sync
for t in page.turns:
    print(t.round, t.outcome, t.model_id, t.candidate_count, t.permutation)
# async: await kx.list_rerank_turns(limit=20)
```

```typescript
// TypeScript SDK
import { KxClient } from "@kortecx/sdk";
const kx = new KxClient("http://127.0.0.1:50051");
const { turns } = await kx.listRerankTurns({ limit: 20 });
turns.forEach((t) => console.log(t.round, t.outcome, t.modelId, t.candidateCount, t.permutation));
```

In the **console**, the **Monitoring → ReRank rounds** panel shows each round's outcome
(`reranked` / `failed_closed` / `pending`), model, candidate count, and permutation.

## Chaining example — grounded chat with rerank

```sh
# 1. Ingest a dataset
kx datasets ingest kb ./docs/*.md

# 2. Serve with the LLM rerank on
KX_SERVE_RAG_LLM_RERANK=1 kx serve --features inference,hnsw &

# 3. Ask a grounded question (chat-rag) — the answer reads the reranked passages
kx apps run kx/recipes/chat-rag --dataset kb --prompt "how does recovery work?"

# 4. Audit the rerank that fired
kx rerank list --limit 1
```

See also: [Agentic RAG](./agentic-rag.md) · [Data Lab](./datasets.md) ·
[Agents & reasoning](./agent-runner.md) · [Local inference engines](./local-inference-engines.md).
