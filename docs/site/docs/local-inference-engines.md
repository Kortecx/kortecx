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

Whichever engine serves a model is shown everywhere: `kx models list` prints an
engine badge (`[text · ollama]`), the SDKs expose `ModelSummary.engine`, and the
**Models** view shows an engine badge per card. The engine is a display/audit field
only — it never authorizes a route (SN-8), and it is **never journaled**, so the
canonical projection digest is unaffected by which engine answered.

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
# gemma3:12b  [text · ollama]  ctx=0  gemma3:12b  (serving)
```

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
