---
id: local-inference-engines
title: Local inference engines
sidebar_label: Local inference engines
description: Serve local models with zero friction via Ollama, or the self-contained llama.cpp path — both ride the same InferenceBackend seam.
---

# Local inference engines

`kx serve` runs models through a **pluggable inference engine** behind one seam
(`InferenceBackend`). There are two local options, and you can run **both at once**
— each model is served by whichever engine owns it.

| Engine | Install cost | Best for |
|---|---|---|
| **Ollama** (recommended) | a precompiled installer; no C++ toolchain | zero-friction onboarding, GPU on every platform |
| **llama.cpp** (`--features inference`) | builds from source (CMake + clang) | a single self-contained binary, no background daemon |

The **prebuilt binary** (`curl | install.sh`) ships the serve engine, so it
**auto-detects a running Ollama daemon and serves local models out of the box** —
no build, no C++ toolchain. llama.cpp is the opt-in `--features inference` build.

Whichever engine serves a model is shown everywhere: `kx models list` prints an
engine badge (`[text · ollama]`), the SDKs expose `ModelSummary.engine`, and the
**Models** view shows an engine badge per card. The engine is a display/audit field
only — it never authorizes a route (SN-8), and it is **never journaled**, so the
canonical projection digest is unaffected by which engine answered.

## Which engine? (positioning)

The two engines are **co-equal first-class backends** — pick by what you need, and
switch (or run both) at any time:

- **Ollama — the quick/easy path for agent users.** Zero-friction onboarding (no C++
  toolchain), auto-detected on the loopback port, `ollama pull` model management. The
  default recommendation for *"just run an agent now."*
- **llama.cpp — the performance / parallel / multi-modal path.** A self-contained
  single binary with in-process control (KV-cache capacity, batched serving, vision
  via an mmproj projector). The recommendation when you need throughput, parallelism,
  or multi-modal input.

### Capability parity

Every core capability rides the shared `InferenceBackend` seam, so it behaves the
same on either engine. Where a capability is engine-specific today, it is an honest
gap (a visible "unavailable on this engine"), never a silent one:

| Capability | Ollama | llama.cpp |
|---|---|---|
| Chat + agentic ReAct / tool loop | ✅ | ✅ |
| Model lifecycle (load / offload / residency) | ✅ | ✅ |
| Streaming tokens | ✅ | ✅ |
| Context window surfaced (`kx models list` `ctx=`) | ✅ (`/api/show`) | ✅ (GGUF `n_ctx`) |
| Tool-call parsing (multi-format) | ✅ | ✅ |
| Embeddings / RAG (Datasets) | ✅ (`/api/embed`) | ✅ |
| Vision / multi-modal input | ⏳ planned | ✅ (mmproj) |
| Constrained decode (grammar) | reserved | reserved |

⏳ *planned* capabilities land in a follow-up release; until then the gateway
honest-degrades (e.g. a vision turn on an Ollama-only serve is refused, not faked).

## Option A — Ollama (zero-friction)

[Install Ollama](https://ollama.com), pull a model, and start the gateway:

```sh
ollama pull gemma3:12b      # any tag you like
kx serve --dev-allow-local  # auto-detects Ollama on 127.0.0.1:11434
```

With no GGUF configured, `kx serve` detects a running Ollama on the loopback port,
registers its installed models, and serves them through the unchanged agentic loop
(chat, tools/ReAct, the model-driven topology loop). If Ollama is **not** running,
the gateway prints a one-line hint and serves model-free — it never installs Ollama
for you.

```sh
kx models list
# gemma3:12b  [text · ollama]  ctx=131072  gemma3:12b  (serving)
```

The `ctx=` window is read from the daemon's `/api/show` (the model's declared
context length) — the same kind of number the llama.cpp path reads from the GGUF.
It is best-effort: if the daemon doesn't report one, `ctx=0` is shown rather than a
fabricated value.

### Configuration (operator-only, SN-8)

The Ollama endpoint is **operator config** — never model-, client-, or
Mote-controlled. No warrant or recipe parameter can redirect the engine.

| Env var | Default | Meaning |
|---|---|---|
| `KX_SERVE_OLLAMA` | `auto` | `auto` (serve Ollama only when no GGUF is set), `1`/`on` (always), `off` |
| `KX_SERVE_OLLAMA_URL` | `http://127.0.0.1:11434` | the daemon endpoint (**loopback only** by default) |
| `KX_SERVE_OLLAMA_MODELS` | *(all)* | a comma/`;`/newline tag allowlist |
| `KX_SERVE_OLLAMA_ALLOW_REMOTE` | *(unset)* | set `1` to permit a **non-loopback** URL (deny-by-default) |

A non-loopback `KX_SERVE_OLLAMA_URL` is **refused** unless you explicitly opt in
with `KX_SERVE_OLLAMA_ALLOW_REMOTE=1` — the gateway will not silently dial a remote
host.

## Option B — llama.cpp (self-contained)

The in-process llama.cpp backend needs no background daemon but **builds from
source** (a C++ toolchain: CMake + clang/libclang) and a local GGUF file:

```sh
cargo install kx-cli --features inference
KX_SERVE_MODEL_GGUF=/path/to/gemma-3-12b-it-q4_k_m.gguf kx serve --dev-allow-local
```

`kx models list` then tags those models `[text · llamacpp]`. See
[Models](./models.md) for the local model lifecycle (register N, load / offload,
per-model routing) — it is identical regardless of engine.

## Embeddings & RAG (datasets)

The datasets / RAG **server-embed** path (text-only ingest + query) works on **either
engine** — it routes through the same `RoutingBackend`, embedding via the in-process
llama.cpp backend or an Ollama daemon, whichever serves the embed model. (Without an
embed model the FFI-free **client-vector** path still works: supply vectors yourself.)

By default the embedder is the **primary chat model**. To use a dedicated
embedding model, set `KX_SERVE_EMBED_MODEL` (operator config, SN-8 — never
client-chosen):

| Env var | Default | Meaning |
|---|---|---|
| `KX_SERVE_EMBED_MODEL` | *(primary model)* | the model used to embed dataset text. **Must be an embedding-capable model** — e.g. `embeddinggemma` on Ollama (`ollama pull embeddinggemma`), or an embedding GGUF on llama.cpp. A non-embedding model is accepted at startup but fails on the first embed. |

```sh
ollama pull gemma3:12b && ollama pull embeddinggemma
KX_SERVE_EMBED_MODEL=embeddinggemma kx serve --dev-allow-local --features hnsw
kx info        # shows: embed  embeddinggemma (datasets/RAG)
kx models list # the embed model carries an (embed) marker
```

Two parity notes:

- **Pooling.** Ollama applies the embedding model's **native** pooling; the `pooling`
  argument is advisory there (llama.cpp honours it). For mean-pooled embedding models
  (the common case) the results match.
- **Dimension is fixed per dataset.** A dataset's vector dimension is set by its first
  insert. Embedding models of different dimensions are **not** interchangeable within
  one dataset — changing `KX_SERVE_EMBED_MODEL` for an existing corpus requires a fresh
  dataset (a dimension mismatch is refused loudly, never silently corrupting results).

## Running both

If a GGUF is configured **and** Ollama is reachable (`KX_SERVE_OLLAMA=1`), the
gateway serves the **union**: the GGUF model keeps the default chat route, and the
Ollama models are reachable through their per-model recipe handles. A dispatch
routes to the first engine that serves the requested model id (llama.cpp first when
a GGUF is configured); model ids never collide in practice (GGUF-stem ids vs Ollama
tags like `gemma3:12b`).

## What's served where (OSS vs Cloud)

Both local engines are OSS — they run correctly on a single system. GPU-batched,
multi-tenant, or cross-network model serving (vLLM / SGLang / bring-your-own-key
providers) is a managed-Cloud capability.
