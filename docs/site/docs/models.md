---
id: models
title: Models
sidebar_label: Models
description: Discover the models serving a Kortecx gateway — a read-only, display-only catalog.
---

# Models

The **Models** view is a read-only catalog of the models serving a gateway. Open
it from the sidebar (under **Tools**) or list the same data from the CLI:

```sh
kx models list
```

Each model shows its id, modalities (`text` / `image` / `audio` / `video`), a
`serving` badge (the primary/default route), a **`loaded`** badge (whether it is
resident in RAM right now), an **`(embed)`** marker on the configured datasets/RAG
embedder (`KX_SERVE_EMBED_MODEL` else the primary — see
[Embeddings & RAG](./local-inference-engines.md)), the served context window, and a
host-synthesized description. The data comes straight from the gateway's `ListModels`
RPC — nothing here is fabricated.

## Local model lifecycle: register N, load / offload, route

A serving gateway can register **several** local GGUFs and let you warm or evict
them on demand. Register the set at startup — the primary `KX_SERVE_MODEL_GGUF`
plus any extras in `KX_SERVE_MODELS` (a `;`- or newline-separated list of absolute
GGUF paths):

```sh
KX_SERVE_MODEL_GGUF=/abs/gemma-4-12b-it-q4_k_m.gguf \
KX_SERVE_MODELS=/abs/qwen2.5-3b-instruct-q4_k_m.gguf \
  kx serve --listen 127.0.0.1:50151 --journal kx.db --content blobs
```

The registered set is **fixed at startup** (server-controlled). `load` / `offload`
only manage which registered models are resident in RAM — they never register a
new path:

```sh
kx models load    kx-serve:qwen2.5-3b-instruct-q4_k_m   # warm into RAM (real load)
kx models offload kx-serve:gemma-4-12b-it-q4_k_m        # evict from RAM (frees it)
```

- **Sequential swap.** A single owner thread holds the models; the loaded-model
  cache keeps `KX_SERVE_CACHE_CAPACITY` models resident (default **2**). Loading
  past the cap honestly evicts the oldest — sizing matters (a 12B model is ~7 GB).
- **Lazy by default.** A model loads on first use; set `KX_SERVE_WARM_ON_START=1`
  to warm the primary at startup (a warmer first chat).
- **Fail-closed.** Loading or offloading an **unregistered** id returns `not found`
  — never a load of an arbitrary path.
- **Route a chat turn to a chosen model.** Each registered model has its own chat
  recipe; the model picker in **New Chat** routes a turn to the selected model, and
  every model is prompted with its OWN chat template (below).

Load / offload are **ephemeral**: residency is RAM state that rebuilds empty on
restart, written to no journal. The same controls are available in the SDKs
(`load_model` / `offload_model` in Python, `loadModel` / `offloadModel` in
TypeScript) and as Load / Offload chips in the console **Models** view.

## Listing never routes a model (SN-8)

Listing a model **grants nothing**. Selecting a model to run is always a
server-validated recipe parameter, never an action taken from this view. The
**load / offload** controls manage only RAM **residency** within the
server-registered set — they never authorize a model route or register a new path.
Authorization stays the runtime's, never a client choice (see [Security → model
proposes, runtime
enforces](./security.md#model-proposes-runtime-enforces)).

## Honest empty & degraded states

- **No models (FFI-free serve).** A gateway built without an inference backend
  serves no model and `ListModels` returns an **empty list** — the view shows an
  honest empty state, not an error. Start a model-serving gateway with
  `KX_SERVE_MODEL_GGUF` set (see the [Quickstart](./quickstart.md)).
- **Older gateway.** A gateway that predates `ListModels` degrades to a
  "not wired" notice rather than a blank screen.

## Local models & prompt formatting

A model-serving gateway loads a local GGUF named by `KX_SERVE_MODEL_GGUF` (plus an
optional `KX_SERVE_MMPROJ_GGUF` vision projector). The gateway is
**model-agnostic**: every model is prompted with its OWN chat template — the
gateway applies the GGUF's embedded `chat_template` through llama.cpp where it
renders, and falls back to a built-in per-architecture template for models
llama.cpp cannot render, so a model is never fed another model's format. A
recipe's structured reply is normalized symmetrically: a leading reasoning block
or a Markdown code fence around the JSON is stripped before the runtime parses
it, fail-closed.

The recommended local model is **Gemma-4-12B** (Apache-2.0; omni — `text` +
`image`). A tiny public **Qwen3-0.6B** stand-in backs the
[Quickstart](./quickstart.md) and CI. Pull Gemma locally with `just
fetch-gemma-model` and serve it (text + vision) with `just review-serve-gemma`.

Because every model is templated with its own format, registering models from
**different families** works transparently — e.g. Gemma-4 (native
`call:NAME{…}` tool calls) alongside Qwen2.5-3B-Instruct (Hermes
`<tool_call>{…}</tool_call>`). The runtime parses each model's tool-call format
fail-closed. Pull a small second model with `just fetch-2nd-model`.

## Cloud & coming soon

Connecting a managed cloud provider (vendor keys + OAuth) is a **Cloud**
capability, and pulling a model locally is **coming soon** — both render as
disabled cards in the view. They are never faked as local actions.
