---
id: parallel-execution
title: Parallel execution & workers
sidebar_label: Parallel execution
description: Run authored parallelism for real — a bounded local worker pool executes independent agents/tools/steps concurrently while the coordinator stays the sole journal writer, so exactly-once, crash-recovery, and the canonical digest are all unchanged.
---

# Parallel execution & workers

When you author parallelism — a [swarm](./swarms.md), a `parallel(...)` fan-out, a
`fan_out_gather`, or any DAG with independent branches — `kx serve` runs it through a
**bounded local worker pool**. Each worker independently leases ready work, runs it, and
proposes the commit; the **coordinator stays the single journal writer**, so the durable
guarantees are untouched:

- **exactly-once** — commit is first-wins dedup-by-key; two workers can never double-commit.
- **crash-recovery** — a crashed worker's in-flight work is re-offered after the liveness
  window; a mid-run crash re-folds to the identical state.
- **deterministic digest** — the projection digest sorts committed facts by identity, so
  **worker/execution order is not an input**. The same workflow at any pool size commits the
  identical facts (proven by the `pool_determinism` gate across pool ∈ {1, 2, 4}).

> **A serve-time knob, not a chain method.** The pool is *infrastructure* — you set it when
> you launch the server, not on a `flow()`. A client submitting to a shared server does not
> dictate that server's concurrency. So there is intentionally **no `.pool()` on a Flow**;
> the topology you author is orthogonal to how many workers drain it.

## Set the pool size

```bash
# 4 concurrent workers (default is 1 = the historical single worker).
kx serve --workers 4

# or via the environment (the CLI flag wins if both are set):
KX_WORKERS=4 kx serve
KX_SERVE_WORKER_POOL=4 kx serve
```

`--workers 1` (the default) is **byte-identical** to a serve with no pool at all. `--max-lease`
is a *different* knob — it bounds how many ready Motes one worker pulls per poll; under a
pool the per-worker lease is spread automatically so no single worker hoards the ready set.

## What a pool actually parallelizes

| Work class | Concurrency under `--workers N` |
|---|---|
| **Pure** (deterministic compute) | Truly concurrent — up to N at once. |
| **Tool / IO** (MCP tools, HTTP connectors, retrieval) | Truly concurrent — up to N at once. |
| **Model inference** (llama.cpp) | **Serializes** on the one in-process model owner thread — `--workers` overlaps its *tool/IO* turns but not the decode itself. |
| **Model inference** (Ollama) | **Concurrent requests, but GPU-bound throughput** — each worker fires an independent request and the daemon serves up to `OLLAMA_NUM_PARALLEL` at once, so the *requests* overlap. Whether that speeds up wall-clock depends on GPU headroom (see the note below): a model that already saturates the GPU will not decode faster in parallel. |

So a **swarm on Ollama** can serve its agents' requests concurrently (up to
`OLLAMA_NUM_PARALLEL`), while a swarm on the embedded **llama.cpp** engine takes model turns
on the single owner thread. On **both**, `--workers` cleanly overlaps the agents' Pure/tool/IO
work — see the GPU note below for what concurrency means for the *decode* itself.

> **Concurrent *decode* on a single local GPU is GPU-bound (measured, both engines).** A large
> model saturates one GPU, so decoding two requests at once gives ~no throughput gain — and a
> fan-out of decode-only agents can even run *slower* under a pool (the requests contend for the
> same device). On an Apple M3 with a 12B model we measured: llama.cpp two-context concurrent
> decode ≈ 0.8–1.0× vs. sequential; Ollama two concurrent requests ≈ 1.06×; and a 4-agent
> decode-only swarm at `--workers 4` was *slower* than `--workers 1`. (Raising
> `OLLAMA_NUM_PARALLEL` too high also backfires: reserving N× the KV cache can push a large model
> off the GPU into CPU offload — keep it at ~2 for a 12B on ~16 GB.) The takeaway: **the pool's
> wall-clock win is Pure/IO/tool overlap, not concurrent decode.** For decode-heavy local swarms
> keep `--workers` at 1–2; concurrent *inference* throughput needs a smaller model (GPU
> headroom), more GPUs, or a hosted backend. This is why the embedded llama.cpp engine keeps a
> single model owner thread rather than shipping a multi-lane inference pool.

## Back-pressure

Execution is **pull-based**: work becomes *ready* the instant its inputs commit, but nothing
runs until a worker leases it. A fan-out of 100 agents therefore runs **N at a time** with
the rest queued in the coordinator's ready-set — it never stampedes local resources by firing
everything at once. Model inference is additionally queued at the owner thread.

To bound a *hung* tool (an external MCP/HTTP call that never returns and would otherwise pin a
worker's slot), set an optional per-Mote deadline:

```bash
# cancel + retry (then dead-letter) a tool/IO dispatch that exceeds 120s. Default: off.
KX_SERVE_TOOL_DEADLINE_SECS=120 kx serve --workers 4
```

The deadline is a live wall-clock bound (never a journaled fact); a timed-out dispatch is
retried within the worker's failure budget and then dead-lettered, so one stuck tool cannot
wedge the pool.

## See the configured pool

```bash
kx status            # → limits  workers 4 · max_lease 16 · content_max_bytes …
kx status --json     # → { …, "worker_pool": 4, … }
```

```python
import kortecx as kx
info = kx.default_client().get_server_info()
print(info.effective_worker_pool)   # 4  (an old server reports 1)
```

```ts
import * as kx from "@kortecx/sdk";
const info = await kx.defaultClient().getServerInfo();
console.log(info.effectiveWorkerPool); // 4
```

The console's **Settings** panel shows the same figure ("Worker pool"), and **Monitoring**
surfaces live in-flight execution.

## Choosing a size

- **Pure / tool / IO-heavy** workloads (swarms of retrieval or connector calls): scale
  `--workers` toward your core count — these overlap cleanly and are where the pool pays off.
- **Model-inference-heavy** swarms (agents that mostly decode): keep `--workers` **low (1–2)**
  on a single local GPU. Extra workers overlap only the tool/IO *around* each decode, and a
  decode-bound swarm can run *slower* under a large pool because the agents contend for the one
  GPU (measured above). Concurrent decode throughput needs GPU headroom (a smaller model), more
  GPUs, or a hosted backend.
- **Default 1**: safest for a laptop where a single model already saturates the GPU/cores.
