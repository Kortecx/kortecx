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
`serving` badge (whether it backs the live loop), the served context window, and a
host-synthesized description. The data comes straight from the gateway's
`ListModels` RPC — nothing here is fabricated.

## Listing never routes a model (SN-8)

Listing a model **grants nothing**. Selecting a model to run is always a
server-validated recipe parameter, never an action taken from this view — so the
catalog has no "use model" control. Authorization stays the runtime's, never a
client choice (see [Security → model proposes, runtime
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

## Cloud & coming soon

Connecting a managed cloud provider (vendor keys + OAuth) is a **Cloud**
capability, and pulling a model locally is **coming soon** — both render as
disabled cards in the view. They are never faked as local actions.
