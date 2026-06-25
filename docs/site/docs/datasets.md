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
```

Add `--json` to any subcommand for a machine-readable form (byte-shape parity with
the SDKs).

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
