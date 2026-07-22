---
id: intro
title: Introduction
sidebar_label: Introduction
slug: /intro
description: What Kortecx is — a durable, governed runtime for AI agents.
---

# Kortecx

> The durable runtime for AI agents. **Knowledge → Intelligence.**

Kortecx runs AI agents you can trust with real work. One small binary gives you
**durable, exactly-once agentic execution** — live agent loops that plan,
re-plan, self-check, and call tools; reusable **Blueprints**; **RAG datasets**;
**local LLM inference**; a built-in **web console**; and **Python / TypeScript
SDKs** — all over an append-only journal that survives crashes and never runs a
world-touching step twice.

```bash
curl -fsSL https://raw.githubusercontent.com/Kortecx/kortecx/main/scripts/install.sh | sh
kx serve --journal /tmp/kx.db --content /tmp/kx-content --dev-allow-local
# → gRPC 127.0.0.1:50151 · events ws://127.0.0.1:50152 · web console http://127.0.0.1:8888
```

## What you get

| Capability | What it does |
|---|---|
| **Exactly-once agentic runs** | Every step (a [*Mote*](./concepts.md#mote)) commits durably to an append-only [journal](./concepts.md#journal--journalentry); crashes replay from committed work — a step that touched the world is re-read, never re-run. |
| **The live agent loop** | Models **plan** topology, **re-plan** on failure, pass **critic** gates, and run **ReAct turns with real MCP tools** — all inside `kx serve`, all crash-safe. |
| **Blueprints** | Reusable, parameterized workflows published by handle — pick one, fill its typed inputs, run it, watch the live DAG. |
| **Local LLM inference** | Bring any fit GGUF model; on-device llama.cpp (Metal/CPU) drives chat and the agent loop — no API keys, no egress. |
| **Datasets & RAG** | Ingest documents, search by vector similarity, ground agent runs — durable, content-addressed corpora. |
| **Live events & time-travel** | Stream every state change as it commits; scrub any run back to any point in its history. |
| **Teams & grants** | Durable membership + asset grants with resolved-warrant views. |
| **The web console** | All of the above in a browser — served by `kx` itself, zero extra setup. |

Every capability is reachable from the **CLI**, the **Python and TypeScript
SDKs**, and the **web console** — same wire, same guarantees.

## The core idea: exactly-once, proven

Kortecx is built around one durable spine. Every step is content-addressed and
committed to an append-only log before its result is trusted. On a crash, the
runtime re-folds that log to rebuild state and resumes: deterministic steps may
be recomputed, but any step that touched the outside world is **served from its
committed result, never re-run**.

You can demonstrate this property directly — run the canonical demo, crash it
mid-commit, replay, and observe the identical digest. See the
[Quickstart](./quickstart.md#prove-exactly-once).

## Where to go next

- **[Quickstart](./quickstart.md)** — install `kx`, start the runtime, run your
  first Blueprint and chain from the CLI and both SDKs.
- **[Concepts](./concepts.md)** — the vocabulary: Mote, Journal, Projection,
  Warrant, Chain, and the guarantees they encode.
- **[Chains DSL reference](./chains/dsl-reference.md)** — the operator grammar
  for composing task handles into a DAG, with worked examples.
- **[Security](./security.md)** — server-built warrants, the
  model-proposes / runtime-enforces boundary, and the deny-by-default defaults.

## Status

Kortecx is in **early development** (pre-1.0). The durability spine and the live
agent loop are feature-complete; surfaces are being filled in across the CLI,
SDKs, and console. Pin a release tag for anything you keep. Built in the open at
[Kortecx/kortecx](https://github.com/Kortecx/kortecx) under the Sustainable Use License (fair-code).
