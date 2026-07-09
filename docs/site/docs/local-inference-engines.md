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
| Model switching (active default, `kx models use`) | ✅ | ✅ |
| Model download (`kx models pull`, runtime-register) | ✅ (`/api/pull`) | ✅ (direct GGUF URL) |
| Streaming tokens | ✅ | ✅ |
| Context window surfaced (`kx models list` `ctx=`) | ✅ (`/api/show`) | ✅ (GGUF `n_ctx`) |
| Tool-call parsing (multi-format) | ✅ | ✅ |
| Embeddings / RAG (Datasets) | ✅ (`/api/embed`) | ✅ |
| Vision / multi-modal input (image→text, OCR) | ✅ (vision tags) | ✅ (mmproj) |
| Agentic vision (image carried across the ReAct loop) | ✅ | ✅ |
| Constrained tool-calling (grammar) | parser-based¹ | ✅ (lazy GBNF) |
| Constrained listwise rerank (permutation) | ✅ (strict `format`) | parser-based² |

A vision turn on a **text-only model** (either engine) is refused, not faked — the
gateway never answers about an image a non-vision model cannot see. ⏳ *planned*
capabilities land in a follow-up release.

¹ Ollama has no lazy/triggered grammar, so a tool turn (which may answer in prose OR
call a tool) honest-degrades to the multi-format parser rather than a whole-response
`format`. A **rerank** turn is different — its entire output is a permutation, so a
strict whole-response `format` is applied on Ollama. For a **tool-first** recipe you can
opt in to a strict tool-call `format` on Ollama tool turns with
`KX_SERVE_OLLAMA_TOOL_FORMAT=1` (default off) — but that is **tool-required**: the model
can no longer answer in prose on a tool turn, so a multi-turn agent that must answer would
dead-letter. Leave it off for general agents; llama.cpp is unaffected (its lazy GBNF lets
prose flow until the tool-call opener).

² On llama.cpp the rerank also relies on the model + the fail-closed parser: its
char-level grammar sampler crashes mid-decode when constraining a digit-array
permutation against some tokenizers (e.g. Gemma's), so the model emits a clean array
after its reasoning and `parse_permutation` strips the preamble + tolerates trailing text.
The rerank prompt is rendered through the served model's **chat template** — an
un-templated prompt degenerates an instruct model into repetition (a `[3] and and …`
collapse that fails closed to base order). On both engines the model proposes the order
and the runtime enforces it.

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
| `KX_SERVE_OLLAMA_TOOL_FORMAT` | *(off)* | set `1` to force a strict tool-call `format` on Ollama tool turns (**tool-required** — breaks prose answering; tool-first recipes only) |

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

By default the embedder is the **primary chat model** — a generative **decoder**,
which produces weak sentence embeddings (paraphrase mis-rank). For real retrieval
quality, set `KX_SERVE_EMBED_MODEL` to a **dedicated embedding model** (operator
config, SN-8 — never client-chosen). The runtime flags a decoder-as-embedder honestly
(`kx info` / `kx models list` / the Data Lab notice), but never blocks.

| Env var | Default | Meaning |
|---|---|---|
| `KX_SERVE_EMBED_MODEL` | *(primary model)* | the model used to embed dataset text. **Use an embedding model** — e.g. `embeddinggemma` on Ollama (`ollama pull embeddinggemma`), or an embedding GGUF (`nomic-embed-text` / `bge-small`) on llama.cpp. A non-embedding model is accepted at startup but fails on the first embed. |
| `KX_SERVE_WARM_EMBED` | `off` | pre-load the embed model in the background at serve start, so the first ingest doesn't hit a cold model-load timeout (Ollama). Probe-only — never force-pulls a missing model. |

```sh
ollama pull gemma3:12b && ollama pull embeddinggemma
KX_SERVE_EMBED_MODEL=embeddinggemma KX_SERVE_WARM_EMBED=1 \
  kx serve --dev-allow-local --features hnsw
kx info        # shows: embed  embeddinggemma (datasets/RAG)   [no decoder warning]
kx models list # the embed model carries an (embed) marker
```

See [Data Lab → Hybrid retrieval & chunking](./datasets.md) for the `KX_SERVE_RAG_*`
retrieval-tuning knobs (hybrid mode, chunk size, RRF/MMR), which apply on both engines.

Two parity notes:

- **Pooling.** Ollama applies the embedding model's **native** pooling; the `pooling`
  argument is advisory there (llama.cpp honours it). For mean-pooled embedding models
  (the common case) the results match.
- **Dimension is fixed per dataset.** A dataset's vector dimension is set by its first
  insert. Embedding models of different dimensions are **not** interchangeable within
  one dataset — changing `KX_SERVE_EMBED_MODEL` for an existing corpus requires a fresh
  dataset (a dimension mismatch is refused loudly, never silently corrupting results).

## Vision & OCR (image → text)

Both engines serve **image→text** (describe an image, answer a question about it, or
**OCR** — "transcribe the text in this image") over a **vision-capable model**:

- **Ollama** — pull a vision model (e.g. `ollama pull gemma3`); the gateway auto-detects
  it via `/api/show` and serves the `kx/recipes/vision` recipe. The image rides the
  `/api/generate` `images` array.
- **llama.cpp** — point `KX_SERVE_MMPROJ_GGUF` at the model's mmproj projector GGUF
  (alongside the model GGUF). The image is spliced by the mmproj projector.

The image is uploaded to the content store (`PutContent`, ≤16 MiB) and attached by ref;
the raw bytes never enter the prompt text. A vision turn on a **text-only model** is
refused, not faked.

```sh
# Ollama
ollama pull gemma3
kx serve --dev-allow-local
kx chat --image ./receipt.png "Transcribe all the text in this image."   # OCR
kx chat --image ./cat.png     "What is in this picture?"                  # describe

# llama.cpp
KX_SERVE_MMPROJ_GGUF=./gemma-4-mmproj.gguf kx serve --features inference --dev-allow-local
kx chat --image ./cat.png "What is in this picture?"
```

```python
import kortecx as kx
client = kx.KxClient("http://127.0.0.1:50151")
with open("receipt.png", "rb") as f:
    print(client.chat("Transcribe all the text.", image=f.read()))  # bytes → upload
```

```typescript
import { KxClient } from "@kortecx/sdk";
const kx = new KxClient("http://127.0.0.1:50151");
const bytes = new Uint8Array(await (await fetch("/cat.png")).arrayBuffer());
console.log(await kx.chat("What is in this picture?", { image: bytes }));
```

> **Scope.** This is **model-quality VLM-OCR** (a vision model reads the text), not a
> dedicated OCR engine — no bounding boxes or structured table extraction, and quality
> scales with the model and image resolution. `dataset` + `image` together (vision-RAG)
> and image-in-the-agentic-loop are follow-ups.

## Running both

If a GGUF is configured **and** Ollama is reachable (`KX_SERVE_OLLAMA=1`), the
gateway serves the **union**: the GGUF model keeps the default chat route, and the
Ollama models are reachable through their per-model recipe handles. A dispatch
routes to the first engine that serves the requested model id (llama.cpp first when
a GGUF is configured); model ids never collide in practice (GGUF-stem ids vs Ollama
tags like `gemma3:12b`).

## Switching & downloading models

Switch the active default and download new models on either engine without a restart —
see [Models → Switch the active model](./models.md#switch-the-active-model) and
[Pull a model](./models.md#pull-a-model-download--register-no-restart). Downloads are
operator-gated (deny-by-default), configured by:

| Env var | Default | Meaning |
|---|---|---|
| `KX_SERVE_ALLOW_MODEL_PULL` | `off` | the operator opt-in that authorizes `kx models pull` (egress). Off ⇒ every pull is refused. |
| `KX_SERVE_MODEL_PULL_HOSTS` | `huggingface.co` (+ CDNs) | extra allowlisted download hosts for a direct-URL pull (comma-separated). |
| `KX_SERVE_MODELS_DIR` | `<catalog>/models` | where a direct-GGUF download lands. |

Ollama pulls go through the daemon's `/api/pull` (the registry digest verifies them); a
direct GGUF URL is `https`, allowlisted, `/resolve/`-shaped, and SHA-256-verified before
registration (SN-8).

## System requirements & performance

Local inference is memory-bound: the model weights dominate. A model's memory footprint
is roughly its **GGUF/quantized size on disk**, plus a smaller working set for the KV
cache (grows with context length) and the runtime.

| Model class (example) | ~Weights | Fits comfortably on | What to expect |
|---|---|---|---|
| **Tiny** (Qwen3-0.6B, Q4) | ~0.5 GB | any laptop (8 GB) | near-instant, great for tests/CI — but too weak to reliably drive tools/agents |
| **Small** (3–4B, Q4) | ~2–3 GB | 8–16 GB | fast; usable for simple single-step Apps |
| **Mid** (12B, Q4 — e.g. Gemma-4-12B / gemma3:12b) | ~7–8 GB | **16 GB** (close other heavy apps) or 24 GB+ | the practical floor for reliable **agentic** Apps (tool-calling, `reach`, ReAct loops) |
| **Large** (27–32B, Q4/Q8) | ~16–35 GB | 32–64 GB | best quality; needs headroom for context + KV |

**On a 16 GB Mac (Apple Silicon):** a 12B Q4 model is the sweet spot. Plan for **~10 GB
free memory** (≈8 GB weights + KV + runtime) and close other memory-heavy apps. Measured
single-machine wall-clock on an M-series Mac (a cold first response includes the model
load; a single spike, not a percentile): a served single-step App answered in **~15–26 s**
and an agentic (`reach`/tool-firing) App settled its ReAct loop in **~20–34 s** across both
engines. The gateway itself is thin — an **idle `kx serve` uses ~30 MB** (the model is
lazy-loaded on first inference).

**Two memory profiles, same seam:**

- **llama.cpp (`--features inference`)** loads the model **in-process** and memory-maps
  the weights (on Apple Silicon they live in unified/Metal memory). The `kx serve`
  process's resident set undercounts the mmap'd weights, so watch total system memory,
  not just the process RSS.
- **Ollama** runs the model in its **own daemon** (GPU-accelerated), so `kx serve` stays a
  thin gRPC gateway and the weights sit in the Ollama runner. Easiest onboarding; check
  `ollama ps` for the model's memory + GPU use.

**Better hardware lifts every ceiling:** more RAM/VRAM ⇒ larger and less-quantized models
(sharper reasoning + more reliable tool-calling), longer contexts, several models served
at once (`KX_SERVE_MODELS`), and faster generation. If a model is too big for your machine,
pick a smaller quant (Q4 over Q8) or a smaller parameter count — an App's `reach`, tools,
and workflow are unchanged; only the model behind them differs.

> **Tip:** keep your interactive/agent testing on a **12B-class** model — smaller models
> often won't emit a tool call at all, which reads as "the App didn't work" when the real
> cause is model capacity.

## What's served where (OSS vs Cloud)

Both local engines are OSS — they run correctly on a single system. GPU-batched,
multi-tenant, or cross-network model serving (vLLM / SGLang / bring-your-own-key
providers) is a managed-Cloud capability.
