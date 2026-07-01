---
id: agentic-rag
title: Agentic RAG
sidebar_label: Agentic RAG
description: Let a local agent search your datasets autonomously — the model decides when to retrieve, phrases its own query, reads the passages, re-queries, and answers grounded in what it found (the retrieve tool + the react-rag recipe). Plus vision-RAG (ground an image answer on retrieved text).
---

# Agentic RAG

Plain [RAG](./datasets.md) retrieves **once** and folds the passages into the prompt
(`kx/recipes/chat-rag`). **Agentic RAG** hands the agent a **`retrieve` tool** instead:
the model decides *when* to search, *phrases its own query*, reads the passages, **re-queries**
if the first result is thin, and answers grounded in what it found — a `reason → retrieve →
reason` loop over the same hybrid (keyword + semantic) index. The retrieval is durable and
replayable: each retrieve call commits the **ordered content-refs** of the passages it read
(scores stay out — model proposes, the runtime records exact references only).

This is the `kx/recipes/react-rag` recipe (a sibling of [`react`](./agent-runner.md) and
`react-fs`), provisioned automatically on a serve with datasets enabled (the `hnsw` build).
It is reachable from the **one entry point** — `kx` / `KxClient` — exactly like every other
capability.

## The `retrieve` tool

The agent's warrant grants a single read-only built-in tool:

```json
retrieve@1 — {"dataset": <name>, "query": <search text>, "k": <1..64, optional>}
→ ordered passages: [{ "ref": <chunk hash>, "text": ..., "parent_ref": <doc hash>, "chunk_index": N }]
```

It runs the [hybrid retrieval](./datasets.md) path (BM25 + dense, RRF-fused, MMR-reranked),
so a keyword a weak local embedder mis-ranks is still caught. It is **read-only**
(`Readback` — the human-in-the-loop gate auto-proceeds it), has **no egress and no filesystem
scope**, and **fails soft**: an unknown / empty / stale dataset returns an empty observation the
agent reads and recovers from — the loop never dead-letters on a retrieval miss.

## Run it

Point the agent at a dataset with `--dataset`; it does the rest.

**CLI** — `kx agent run --dataset`:

```bash
# Ingest a corpus once (see Data Lab), then:
kx agent run --goal "What does the handbook say about parental leave?" --dataset handbook
# · agentic RAG over dataset 'handbook' — the model searches it with the retrieve tool
# { "answer": "...", "actions": [{ "tool_id": "retrieve", "tool_version": "1", "turn": 0, ... }], ... }
```

The audited `actions` show every `retrieve` call the agent made (and any re-query) in turn order
— the same `ListReactTurns` trace `kx react list` reads back.

**Python** — `client.invoke(REACT_RAG_RECIPE_HANDLE, …)`:

```python
from kortecx import KxClient, REACT_RAG_RECIPE_HANDLE

kx = KxClient("http://127.0.0.1:50151")
answer = kx.invoke(
    REACT_RAG_RECIPE_HANDLE,
    {"instruction": "What does the handbook say about parental leave?", "dataset": "handbook"},
    wait=True,
)
print(answer.text)
# async: await AsyncKxClient(...).invoke(REACT_RAG_RECIPE_HANDLE, {...}, wait=True)
```

**TypeScript** — `client.invoke(REACT_RAG_RECIPE_HANDLE, …)`:

```ts
import { KxClient, REACT_RAG_RECIPE_HANDLE } from "@kortecx/sdk";

const kx = new KxClient("http://127.0.0.1:50151");
const answer = (await kx.invoke(
  REACT_RAG_RECIPE_HANDLE,
  { instruction: "What does the handbook say about parental leave?", dataset: "handbook" },
  { wait: true },
)) as { text?: string };
console.log(answer.text ?? "");
```

**Console** — open **New Chat**, turn on **Agent mode**, and pick a dataset. Each message routes
to `react-rag`; the agent's `retrieve` calls and the passages it read stream into the reasoning
trace, so you can watch it search, re-query, and ground its answer.

> **react-rag vs chat-rag.** Use **chat-rag** (`kx chat --dataset`) for a fast, single-shot
> grounded answer. Use **react-rag** (`kx agent run --dataset`) when the question may need the
> agent to **decide what to search for**, refine the query, or search more than once. The
> production chat path is already hybrid; agentic RAG adds the autonomy.

## Vision-RAG

Combine an **image** and a **dataset** to ground a multimodal answer: the served vision model
answers about the image *while* grounded on the dataset's top-k retrieved **text** passages — in
one generation (`kx/recipes/vision-rag`). Datasets stay text-only (image embeddings are a later
addition); the image is attached, the text is retrieved.

```bash
kx chat --image ./invoice.png --dataset policies "Does this invoice comply with our expense policy?"
# · image + dataset 'policies' — grounding the vision answer on retrieved text
```

```python
kx.chat("Does this invoice comply with our expense policy?",
        image=open("invoice.png", "rb").read(), dataset="policies")
```

```ts
await kx.chat("Does this invoice comply with our expense policy?",
              { image: invoiceBytes, dataset: "policies" });
```

Every surface **honest-degrades**: no dataset → plain agent / plain vision; no vision model →
plain chat — it never silently drops the image or fakes grounding.

## Operator notes

- `kx/recipes/react-rag` is seeded automatically when a model is served **and** datasets are
  available (the `hnsw` build). `kx/recipes/vision-rag` additionally needs an image-capable model.
- Retrieval quality is the [Data Lab](./datasets.md) hybrid pipeline — the same chunking, BM25 +
  dense fusion, MMR rerank, and embed-model recommendation apply. Prefer a dedicated embedding
  model (e.g. `embeddinggemma`) over a decoder LLM for the corpus.
- Everything is **off the canonical projection digest** — the recipes are catalog entries, and the
  committed retrieval fact is the ordered chunk-ref set (scores excluded). A crashed agent recovers
  to its exact recall state.

## Reranking: deterministic MMR now, LLM listwise rerank (authored pipelines)

Two reranking layers improve precision after the BM25 + dense fusion:

- **MMR diversity rerank (deterministic, always available).** Demotes near-duplicate passages
  while preserving the fused relevance order. It runs on every live retrieval path (the Data Lab
  query, `chat-rag`, and the agentic `retrieve` tool) and is controllable per query
  (`--rerank on|off`; see [Data Lab → Hybrid retrieval](./datasets.md#hybrid-retrieval--chunking)).
- **LLM listwise rerank (model-graded, LIVE in `kx serve`).** A model reorders the retrieved
  candidates by emitting a **permutation** of their indices (Ollama applies a strict whole-response
  JSON `format`; llama.cpp relies on the model + parser — see
  [engine notes](./local-inference-engines.md)). It is **fail-closed**: any non-permutation output
  keeps the deterministic order, so a rerank can never reorder into garbage (the model proposes, the
  runtime enforces — SN-8). It completes the RAG quartet **rewrite → retrieve (hybrid) → rerank →
  assemble**. As of RC4c-2b it runs **live in `kx serve`** as a **durable, replayable coordinator
  rerank-turn** — enabled with `KX_SERVE_RAG_LLM_RERANK=1`, applied to both the agentic `retrieve`
  loop (react-rag) and the grounded `chat-rag`/`vision-rag` answer, and recorded as an auditable
  `ReRankRound` fact. See **[LLM rerank](./llm-rerank.md)** for the full contract + the
  CLI/SDK/UI chaining. The deterministic MMR rerank remains the always-on default.

See also: [Data Lab](./datasets.md) · [Agents & reasoning](./agent-runner.md) ·
[Tools](./tools.md) · [Local inference engines](./local-inference-engines.md).
