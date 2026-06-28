---
id: evaluation
title: Evaluation
sidebar_label: Evaluation
description: The measure-first eval harness — a golden-suite regression gate plus a per-run quality readout, scoring agent runs on local OSS models.
---

# Evaluation

kortecx is **measure-first**: agent quality is a number you can gate on, not a vibe. The
`kx-eval` harness scores agentic runs on the local OSS models you already run (Gemma /
Ollama / llama.cpp) and exposes two surfaces from the single `kx` / client entry point:

- **The golden gate** — `kx eval run` scores a versioned **golden suite** against a
  committed baseline and fails closed on any regression. This is the ratchet every
  release change is held against.
- **The per-run quality readout** — `kx eval score <run>` (and `kx.eval` / `client.eval`)
  summarises one *live* run's trajectory: did it reach an answer, how many turns and
  tool-calls it spent, how much of its budget it burned, how many proposals were
  rejected.

A score is an integer **per-mille** (`0..=1000`); a gate pass/fail is an exact integer
comparison, never a float.

## What it measures

The golden suite scores five Gate metrics:

| Metric | Question it answers |
| --- | --- |
| `task_success` | Did the run reach the expected terminal (answer / clean dead-letter) with the right answer? |
| `tool_call_f1` | Did it call the right tools (order-tolerant, ToolBatch-aware)? |
| `groundedness` | Are the answer's claims traceable to retrieved docs? |
| `loop_efficiency` | How economically did it reach the terminal (turns + tool-calls vs ideal)? |
| `format_coverage` | Does the runtime's parser decode tool calls across the shapes different models emit (JSON-envelope, Gemma brace/paren, Llama tag, Qwen XML, markerless, OpenAI array, …)? |

## CLI

```bash
# Run the golden gate locally (no gateway, no model — deterministic, cannot flake).
kx eval run

# Allow a little slack (per-mille) before a Gate counts as a regression.
kx eval run --tolerance 20

# Machine-readable.
kx eval run --json

# Score one live run's trajectory quality (via the gateway).
kx eval score 00112233445566778899aabbccddeeff
```

`kx eval run` exits non-zero on any regression or corpus drift — drop it into CI exactly
like `just eval`.

## SDK

The per-run readout chains off the single client, alongside `kx.cost` and
`kx.approvals`:

```python
from kortecx import KxClient

with KxClient("http://127.0.0.1:50151") as kx:
    q = kx.eval.score_run("00112233445566778899aabbccddeeff")
    print(q.terminal, q.reached_answer, q.turns_used, q.rejections)
```

```typescript
import { KxClient } from "@kortecx/client";

const kx = new KxClient("http://127.0.0.1:50151");
const q = await kx.eval.scoreRun("00112233445566778899aabbccddeeff");
console.log(q.terminal, q.reachedAnswer, q.turnsUsed, q.rejections);
```

## How it works

- **Two tiers, one scorer.** Every scorer is a pure function of a *transcript* — the
  reduced record of a run's turns, answer, and retrieved docs. The golden gate builds
  transcripts from **scripted fixtures** (deterministic, no model, CI-required); the
  per-run readout builds one from a **live run** (advisory). The same scorer code serves
  both, so the gate and the live readout can never disagree.
- **The baseline is committed.** `kx eval run` compares against an embedded baseline
  captured by `kx eval run --update-baseline`. A corpus change shifts its content digest
  and the gate fails closed until the baseline is deliberately re-captured — a
  measurement-contract change is never silent.
- **Off the critical path.** Eval reads committed facts and scores them. It never writes
  a fact, never feeds the canonical projection digest, and runs only at dev/CI time.

## Determinism — the precise scope

The golden gate is byte-deterministic (scripted fixtures, integer scoring) and is the
regression ratchet. Live-model numbers (latency, real-model task success) are
**advisory** — recorded to the private benchmark trend, never a hard CI assertion —
because local OSS-model sampling and quantisation are not bit-reproducible across
machines. See [Observability](./observability.md) for the per-run telemetry the readout
complements.
