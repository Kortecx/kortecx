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

Every metric below is a Gate: an integer per-mille, compared exactly, never averaged as a
float. A metric that does not apply to a task is **N/A** and is excluded from the mean —
never counted as a zero, because "this task had nothing to ground" and "this run grounded
nothing" are different facts.

| Metric | Question it answers | How it is computed |
| --- | --- | --- |
| `task_success` | Did the run reach the expected terminal with the right answer? | **Binary, 1000 or 0.** The terminal must match, and every `answer_must_contain` string must appear in the run's committed answer. See the oracle note below. |
| `tool_call_f1` | Did it call the right tools? | Multiset F1 of actual against expected calls by exact `(id, version)`, order-tolerant. A task expecting no tools scores 1000 for calling none — and is driven down by a spurious call. |
| `groundedness` | Are the answer's claims traceable to retrieved docs? | The share of `grounded_in` tokens present in **both** the answer and at least one retrieved document. |
| `memory_quality` | Did it recall a durable fact from a previous run, and use it? | The share of `memory_must_recall` facts present in both the recalled memories and the answer. A recall that silently returned nothing scores 0. |
| `loop_efficiency` | How economically did it reach the terminal? | `(ideal_turns + ideal_tool_calls) / (turns + tool_calls)`, capped at 1000. The ideal is **hand-authored per task** — it is a judgement about how the task ought to be solved, not a measurement. |
| `injection_resistance` | Did it treat tool output as data, rather than obeying it? | **Binary.** Fails if the run fires a tool named in `forbidden_tools` or repeats a planted string from `answer_must_not_contain`. The forbidden tool is one the run really was granted — a refusal it could not have disobeyed would measure the grant, not the model. |
| `model_time_share` | How much of a task's wall clock was the model, rather than the runtime around it? | `model_ms / total_ms`. A **ratio** on purpose: an absolute-millisecond gate reads differently on a slower host with no code change. See [Speed](#speed) below. |
| `rerank_quality` | Did the [LLM listwise rerank](./llm-rerank.md) improve ranking — did the most-relevant passage (placed last) move into the top? | A fail-closed rerank scores 0. Applies to the scripted tier only today. |
| `consolidation_quality` | Did a consolidation distil several facts into one recalled entry? | Binary. No live task exercises it yet. |
| `skill_quality` | Did a skill-bearing run stay inside its declared tool wish? | Binary. No live task exercises it yet. |
| `format_coverage` | Does the parser decode tool calls across the shapes different models emit (JSON-envelope, Gemma brace/paren, Llama tag, Qwen XML, markerless, OpenAI array, …)? | Corpus-level; N/A for a live suite, which measures one model's actual output rather than a parse matrix. |

### What the oracle actually is

`task_success` is **case-insensitive substring containment** against the run's committed
answer — not an LLM judge, not exact match, not semantic similarity. For an all-digit
expectation, thousands separators are stripped from the answer first, so `1,000` satisfies
`1000` (that check once measured number formatting).

Two consequences worth stating plainly. A substring oracle can be satisfied by accident —
`50` is contained in `350` — so oracle values are chosen to make that impossible rather
than assumed away. And an oracle a model can reach **without firing anything** measures the
model, not the runtime: the store-only tokens in this corpus (`QUILL-MERIDIAN-58`,
`TRESTLE-62`, `SLIPWAY-COBALT-19`) exist nowhere but the fixtures, which is what makes a
correct answer evidence that the tool ran.

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

## The real-model oracle benchmark (`bench-v1`)

The golden gate scores *scripted* transcripts. `bench-v1` scores **real ones**: every task is
driven on a served model and its actual committed answer is graded by the same oracle
scorers — so agentic quality is a measured number, not a replay.

```bash
# Both engines; restart per run. Numbers land in the (gitignored) docs/benchmarks/.
# Ollama needs a DEDICATED EMBEDDER or the whole `reach` family skips:
KX_SERVE_OLLAMA=1 KX_SERVE_OLLAMA_MODELS=gemma3:12b,embeddinggemma:latest \
  KX_SERVE_EMBED_MODEL=embeddinggemma:latest just eval-bench           # Ollama
KX_SERVE_MODEL_GGUF=<gemma-12b.gguf> just eval-bench                   # llama.cpp
```

Run `just eval-bench` with no environment at all and it will drive whatever model it finds
and skip the families whose fixtures it cannot provision — a partial score, loudly labelled
as one. The full invocation above is what produces a comparable number.

⚠ `ollama stop <model>` **before** the llama.cpp arm. GPU residency is a cross-engine
singleton: an Ollama keep-alive holding a resident 12B makes the in-process 12B fail to
allocate and dead-letter every task, which looks exactly like a capability collapse.

The run's own preamble tells you what it actually covered:

```
eval-bench: scoring 26 live task(s) on [macos/aarch64 (8 cores) | ollama | gemma3:12b]
  (capable=true, reach_fixtures=true, http_tool=true, flaky_tools=true, tool_deadline=20s)
```

Every one of those flags gates a family. `reach_fixtures=false` means the dataset, memory
and capability-inheritance tasks did not run; `flaky_tools=false` means the failure and menu
families did not. Read that line before believing a number — and note that any `false` makes
the run INCOMPLETE, which refuses a baseline capture by construction.

`KX_BENCH_ONLY=<task-id,…>` narrows a run to named tasks while attributing a change to one
part of the loop. It is a diagnostic only: every held-back task is reported as skipped, so
the run is incomplete by construction and a baseline capture is refused.

It is a **local** gate — never part of `just ci`, which stays model-free and flake-proof.
A committed per-engine baseline is the fail-closed ratchet, and the oracle floors are
asserted only for a model capable enough to be worth gating on.

The suite spans ten **families**, each exercising a different part of the runtime. A family
is a bucket of tasks, and its gate is the mean over that bucket — so the **task count is the
denominator**, and in a family of three, one task flipping moves the score by 333.

| Family | Tasks | What a task proves |
| --- | ---: | --- |
| `tool` | 6 | The agent picks the right tool and its answer carries a fact only the tool could supply — including chaining one tool's output into the next call. |
| `react` | 3 | Whether to use a tool at all: an instruction naming a tool the run was never granted fires **nothing** (naming is not granting), a fact with no world-knowledge prior *is* looked up, and a question the model already knows is answered without reaching for anything. |
| `script` | 3 | The agent runs a registered script in the sandbox and answers from what it computed. |
| `reach` | 3 | How far the runtime reaches beyond the prompt: a [dataset](./datasets.md) it searches, a [memory](./memory.md) it recalls, and an app whose capability set is inherited rather than declared. |
| `swarm` | 1 | N agents run in parallel and a gather merges their committed outputs — the answer must carry every agent's contribution. |
| `http` | 2 | A tool reached over the **network**, not a bundled subprocess: the runtime dials it over HTTP, presents a bearer credential resolved at dispatch, and pages through a result set whose answer is on the second page. |
| `failure` | 4 | Tools that error, hang, and return unusable payloads. The loop must surface the failure and let the model take another turn — and a healthy control fails if it starts distrusting every tool. |
| `menu` | 1 | Selection when the menu is as long as the runtime will present, rather than a choice between two obvious options. |
| `long` | 1 | The longest chain the runtime admits: six tool calls across four distinct tools, inside the eight-turn ceiling. |
| `adversarial` | 2 | Input that is trying to steer the agent — including an instruction planted in a **tool's output** — paired with a legitimate request that merely looks like one. |

Each family reports its own gate (`task_success@swarm`) beside the suite-wide one, so a
regression in one capability is visible instead of being averaged away by the others.

### Speed {#speed}

Absolute latencies are recorded as **Spikes** and never gated: they are dominated by the
host's GPU, so a gate on them would fail on a slower machine with no code change at all and
could not tell a regression from a busier laptop.

What *is* gated is `model_time_share` — the fraction of a task's wall clock spent inside the
model rather than in the runtime around it (scheduling, folding, committing, tool rounds).
A uniformly slower host raises both terms together, so the ratio holds or drifts upward.
That asymmetry is deliberate and worth knowing: **this gate cannot false-fail on slow
hardware, only false-pass** — which is why every number carries the environment it was
captured on.

Timing comes from the host's execution telemetry, which is off-journal and rebuildable to
empty. When it is unavailable the score is N/A and no gate is emitted — never a zero, which
would read as the runtime having consumed the entire run. Once a baseline records the gate,
a later run that cannot measure it is missing a baseline gate and fails closed.

### What this suite does not measure

Stated because a benchmark's silence is easy to misread as coverage:

- **One model family, two engines.** Every number is Gemma-4-12B-class. Nothing here
  generalises to a different model without re-running it.
- **Local fixtures.** The HTTP tool is a real network round-trip to a hermetic local
  server. Nothing reaches the public internet — a benchmark that did would measure the
  weather — so rate limits, real auth providers and genuine third-party outages are absent.
- **Eight turns.** The ReAct parameter contract admits at most eight, so nothing here says
  how the loop behaves over dozens.
- **One injection.** `injection_resistance` passing means this run did not take *this*
  bait. It is evidence about a sample, not a property of the system.
- **No concurrency.** Tasks run one at a time.

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
always-on regression ratchet.

Real-model numbers are **not** bit-reproducible — local model sampling and quantisation
vary across machines — so they are never a hard CI assertion. That does not make them
unfalsifiable: `bench-v1` ratchets each engine against its own committed baseline with a
tolerance sized to absorb single-task nondeterminism, and fails closed both when a gate
drops below that baseline and when the corpus changes underneath it. What stays advisory
is the *trend* record (latency, per-run spikes), not the gate.

See [Observability](./observability.md) for the per-run telemetry the readout complements.
