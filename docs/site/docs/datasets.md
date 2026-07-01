---
id: datasets
title: Data Lab
sidebar_label: Data Lab
description: Retrieval corpora and a multi-modal viewer — ingest documents, search by meaning, and preview hits (text, JSON, markdown, images, audio, video) in the browser.
---

# Data Lab

The **Data Lab** is Kortecx's OSS data-plane workbench: content-addressed retrieval
corpora (RAG) backed by an in-process HNSW vector index, plus a multi-modal viewer
that renders results — text, JSON, markdown, **images, audio, and video** — directly
in the browser. Open it from the sidebar (under **Data**), or drive the same RPCs
from the CLI and the SDKs.

## Datasets at a glance

A **dataset** is a named retrieval corpus held in the gateway's catalog directory:
content-addressed document bytes plus a rebuilt-on-open HNSW graph (`kx-dataset-hnsw`).
It is **append-only with content-addressed dedup** — re-ingesting identical bytes is
a no-op, and there is **no delete** (the store is durable, off-journal; a corrupt
index simply rebuilds from the rows).

```sh
kx datasets list
kx datasets ingest my-corpus --text "the first document" --text "another doc"
kx datasets ingest my-corpus --file ./notes.md
kx datasets query my-corpus --text "what did we decide?" --k 5
kx datasets query my-corpus --text "what did we decide?" --mode hybrid   # BM25 + dense
```

Add `--json` to any subcommand for a machine-readable form (byte-shape parity with
the SDKs).

## Hybrid retrieval & chunking

Retrieval combines two signals, fused by **Reciprocal Rank Fusion (RRF)** and
diversified by **MMR**:

- **Dense** — embedding (vector) similarity. Good at *meaning* (paraphrase, synonymy).
- **Sparse (BM25)** — keyword/term overlap. Good at *exact terms* a weak sentence
  embedding mis-ranks (names, codes, rare words).

`--mode hybrid` (the default for server-embedded text) runs both legs; `--mode dense`
runs vectors only. The SDKs take a `mode` argument (`RetrievalMode.HYBRID` / `.DENSE`);
the Data Lab search panel exposes a **Hybrid / Dense** chip. Hybrid silently falls
back to dense when there is no query text (the FFI-free client-vector path).

**Per-query rerank override.** MMR diversity rerank follows the operator's
`KX_SERVE_RAG_RERANK` default; override it for a single query with `--rerank on|off`
(CLI), `rerank=True/False` (Py), `{ rerank: true }` (TS), or the **Auto / Rerank / Off**
chip in the Data Lab:

```bash
kx datasets query my-corpus --text "what did we decide?" --mode hybrid --rerank off
```

```python
hits = client.query_dataset("my-corpus", text="…", mode=RetrievalMode.HYBRID, rerank=True)
```

**Chunking.** Server-embedded documents are split into overlapping **passages**
(default ~1000 chars, 200 overlap) before embedding, so a hit is the relevant
*passage*, not a whole document. Each hit carries chunk **provenance** — its parent
document ref and its position (`chunk i/N`) — surfaced in `kx datasets query` and the
Data Lab. Client-vector ingest is never chunked (the client owns granularity). A
dataset's summary shows `chunked` + the distinct `chunk_count` alongside `doc_count`
(parent documents). Existing (pre-chunking) corpora keep working — a whole document is
treated as a single chunk.

:::tip Let an agent search it
The same hybrid pipeline powers **[Agentic RAG](./agentic-rag.md)** — hand a dataset to an
agent (`kx agent run --dataset <name>`) and the model searches it autonomously with the
`retrieve` tool, re-querying until it can answer.
:::

## Embedding: server-side or bring-your-own vectors

Ingest and query are **pluggable** on embedding:

- **Server-embed (text).** The CLI and the UI embed text server-side. This works on
  **either inference engine** — `kx serve --features hnsw,serve-engine` with a reachable
  Ollama (set `KX_SERVE_EMBED_MODEL=embeddinggemma`), or `--features hnsw,inference` with
  a GGUF. The embed model defaults to the primary; override it with `KX_SERVE_EMBED_MODEL`
  (see [Local inference engines → Embeddings & RAG](./local-inference-engines.md)). With
  no embedder the gateway answers `FAILED_PRECONDITION` and the UI shows an actionable
  notice (never a crash).
- **Client vectors (FFI-free).** The SDKs accept a pre-computed `embedding` per
  document/query — the FFI-free path that needs no server model (compute vectors with,
  e.g., HuggingFace `sentence-transformers` in Python or `transformers.js` in the browser).

```python
from kortecx import KxClient, IngestDocument

client = KxClient(endpoint="http://127.0.0.1:50151", token="…")
client.ingest_documents("my-corpus", [IngestDocument(content=b"hello", embedding=[0.1, 0.2, …])])
hits = client.query_dataset("my-corpus", text="greeting", k=5)
```

## Embedding quality — use a dedicated embedder

Retrieval is only as good as the embeddings. A generative **decoder** chat model
(e.g. Gemma) produces weak sentence embeddings that mis-rank paraphrases, so for the
server-embed path we strongly recommend a **dedicated embedding model**:

- **Ollama** — `ollama pull embeddinggemma`, then `KX_SERVE_EMBED_MODEL=embeddinggemma`.
- **llama.cpp** — register a small embedding GGUF (e.g. `nomic-embed-text`,
  `bge-small`) and point `KX_SERVE_EMBED_MODEL` at it.

When the configured embedder is a decoder model, the runtime says so honestly — `kx
info` and `kx models list` flag it, `kx datasets ingest` prints a one-line advisory,
and the Data Lab shows a notice. Retrieval still works (it never blocks); hybrid +
chunking lift quality regardless.

## Operator tuning (`KX_SERVE_RAG_*`)

Retrieval is operator-configurable (never client-chosen — SN-8). All are additive and
default-preserving (unset ⇒ the documented default):

| Env knob | Default | Effect |
| --- | --- | --- |
| `KX_SERVE_RAG_MODE` | `hybrid` | default retrieval mode (`dense` \| `hybrid`) |
| `KX_SERVE_RAG_CHUNK_SIZE` | `1000` | max chunk size (chars) |
| `KX_SERVE_RAG_CHUNK_OVERLAP` | `200` | chunk overlap (chars) |
| `KX_SERVE_RAG_MAX_CHUNKS_PER_DOC` | `0` (unbounded) | per-document chunk cap |
| `KX_SERVE_RAG_RRF_K` | `60` | RRF fusion constant |
| `KX_SERVE_RAG_MMR_LAMBDA` | `7000` | MMR relevance/diversity (basis points) |
| `KX_SERVE_RAG_RERANK` | `on` | MMR diversity rerank on/off |
| `KX_SERVE_RAG_STOPWORDS` | `off` | drop English stopwords in BM25 |
| `KX_SERVE_WARM_EMBED` | `off` | pre-load the embed model at serve start (avoids a cold first-ingest timeout) |

Changing the embed model or the chunk config invalidates an existing server-embedded
corpus: a server-embed query then returns `FAILED_PRECONDITION` (re-ingest to rebuild)
rather than silently mis-ranking. The client-vector path is unaffected.

## Scores are display-only (SN-8)

Every hit carries a similarity `score`, but it is **display-only** — a ranking aid,
never an identity input. The durable retrieval result is the ordered **content-ref
set**, matched downstream by exact hash. A client must never route identity through a
score (the approximate, build-order-sensitive ANN ranking never reaches a `MoteId`).

## Search vs. Discover

The Data Lab search panel has two modes:

- **Search** (`QueryDataset`) returns hits **with their document bytes** — click a hit
  to render it inline through the multi-modal viewer.
- **Discover** (`FuzzyDiscovery`, advisory) is the **fuzzy-in / exact-out** primitive:
  it returns only content-addressed refs + a display-only score. Resolve bytes by the
  exact ref via the SDK — no content is shown in this mode, honestly.

## Viewing media artifacts

The same viewer powers the run-artifact gallery (see
[Reading run outputs](./reading-run-outputs.md)). Bytes are classified by a magic-byte
sniff and rendered from a `blob:` object URL — **never a remote `src`**, so there is
no outbound-fetch surface. Markdown renders through a dependency-free, React-element
renderer (never `innerHTML`); non-renderable payloads fall back to a bounded hex
preview with a download. Large media beyond the inline preview limit offers a download
rather than a broken element.

## OSS vs. Cloud

OSS ships the **view + author + deterministic** half of the data plane: vector
retrieval, content-addressed ingest, the multi-modal viewer, and deterministic data
synthesis. The **managed and agentic** half is **Kortecx Cloud** — surfaced in the UI
as honest, disabled cards (never fakes):

- **LLM data synthesis** — deterministic synthesis runs locally; LLM-driven generation
  is Cloud.
- **SQL · transform · visualize** — query/transform pipelines and chart-grade
  visualization are Cloud.
- **External database** — bring-your-own Postgres / a managed multi-modal data layer
  is Cloud.
- **Analytics & governance** — cross-run analytics, dashboards, and lineage/governance
  are Cloud.

## Degraded states

- **No `hnsw` feature.** A gateway built without `--features hnsw` has no dataset view;
  the RPCs answer `UNIMPLEMENTED` and the surfaces say so honestly (`run kx serve --features hnsw`).
- **No embedder.** Text ingest/query on a serve with no embed model (no Ollama and no
  GGUF) returns `FAILED_PRECONDITION` with actionable guidance — the client-vector path
  is the FFI-free alternative.
