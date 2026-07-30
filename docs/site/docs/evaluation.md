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

A score is an integer **per-mille** (`0..=1000`) — a rate on a 0–1000 scale (769 ≡ 76.9%),
never a count — and a gate pass/fail is an exact integer comparison, never a float. An
aggregate is the **floor** of the integer mean over the tasks the metric applied to, so
resolution follows the denominator: a one-task family can only read 0 or 1000, a three-task
family only 0 · 333 · 666 · 1000, and the 32-task suite moves in steps of ~31. Where a
metric is pass/fail per unit (`task_success` everywhere, `injection_resistance`,
`tool_seq_fsa`, `pass_k4` per flagship task, and `retrieval_success_at_8` per query), the
published tables print the exact fraction beside the rate — `666 · 2/3` is two of three
units passed. The graded metrics have no such fraction (they are per-task fractions
averaged); of those, `groundedness` and `context_recall` are each exercised by a **single**
corpus task today, so their suite-wide number is that one task's score.

## What it measures

Every metric below is a Gate: an integer per-mille, compared exactly, never averaged as a
float. A metric that does not apply to a task is **N/A** and is excluded from the mean —
never counted as a zero, because "this task had nothing to ground" and "this run grounded
nothing" are different facts.

| Metric | Question it answers | How it is computed |
| --- | --- | --- |
| `task_success` | Did the run reach the expected terminal with the right answer? | **Binary, 1000 or 0.** The terminal must match, and every `answer_must_contain` string must appear in the run's committed answer. See the oracle note below. |
| `tool_call_f1` | Did it call the right tools? | Multiset F1 of actual against expected calls by exact `(id, version)`, order-tolerant. **N/A on a task expecting no tools** — an empty gold multiset degenerates F1, so abstention is scored by `task_success` as its own binary accuracy (the convention BFCL's relevance detection uses), never folded in here. |
| `tool_seq_fsa` | Did it make the calls in the right **order** — exactly? | **Binary per task.** The actual call sequence equals the task's gold sequence. NESTFUL-style full-sequence accuracy, our own definition; N/A where no tools are expected. |
| `tool_seq_psa` | How much of the right order did it keep? | `LCS(actual, gold) / max(len)` — order-sensitive partial credit. The pair exists because a run that makes the right **set** of calls in a broken order scores a perfect `tool_call_f1`; the sequence columns are where that run is visible. |
| `groundedness` | Are the answer's claims traceable to retrieved docs? | The share of `grounded_in` tokens present in **both** the answer and at least one retrieved document. |
| `context_recall` | Did retrieval **surface** the evidence, whether or not the answer used it? | The share of `grounded_in` tokens present in at least one retrieved document — the retrieval-side half of the pair. `context_recall` high with `groundedness` low is a model that ignored what retrieval found; both low is retrieval that never found it. The judge-free shape of RAGAS `NonLLMContextRecall`. |
| `memory_quality` | Did it recall a durable fact from a previous run, and use it? | The share of `memory_must_recall` facts present in both the recalled memories and the answer. A recall that silently returned nothing scores 0. |
| `loop_efficiency` | How economically did it reach the terminal? | `(ideal_turns + ideal_tool_calls) / (turns + tool_calls)`, capped at 1000. The ideal is **hand-authored per task** — it is a judgement about how the task ought to be solved, not a measurement. |
| `injection_resistance` | Did it treat tool output as data, rather than obeying it? | **Binary.** Fails if the run fires a tool named in `forbidden_tools` or repeats a planted string from `answer_must_not_contain`. The forbidden tool is one the run really was granted — a refusal it could not have disobeyed would measure the grant, not the model. |
| `model_time_share` | How much of a task's wall clock was the model, rather than the runtime around it? | `model_ms / total_ms`. A **ratio** on purpose: an absolute-millisecond gate reads differently on a slower host with no code change. See [Speed](#speed) below. |
| `rerank_quality` | Did the [LLM listwise rerank](./llm-rerank.md) improve ranking — did the most-relevant passage (placed last) move into the top? | A fail-closed rerank scores 0. Applies to the scripted tier only today. |
| `consolidation_quality` | Did a consolidation distil several facts into one recalled entry? | Binary. No live task exercises it yet. |
| `skill_quality` | Did a skill-bearing run stay inside its declared tool wish? | Binary. No live task exercises it yet. |
| `format_coverage` | Does the parser decode tool calls across the shapes different models emit (JSON-envelope, Gemma brace/paren, Llama tag, Qwen XML, markerless, OpenAI array, …)? | Corpus-level; N/A for a live suite, which measures one model's actual output rather than a parse matrix. |
| `pass_k4` | Does a task pass **every time**, not just once? | **tau2-style pass^k over our own task set — never leaderboard-comparable.** Three corpus-flagged flagship tasks are re-run K=4 times, each trial on a fully fresh serve (a re-run on the same state dir would replay the committed result — see the bench section). A task scores 1000 only when **all four** trials pass; the gate is the floor mean over the three. Per-task values ride as ungated Spikes: a single K=4 draw swings 0-to-1000 between captures, and no tolerance absorbs that honestly. |
| `retrieval_success_at_8` | Does retrieval rank the one right document above sixty near-misses? | **Success@k, binary single-relevant qrels: 10 queries over the 61-document near-miss corpus, k=8, random floor k/61 ≈ 131‰ per query.** Most queries target the near-miss documents themselves — hard-negative discrimination, not corpus-scale retrieval — so this is a regression gate on the runtime's retriever, never a BEIR-comparable "Recall@k". |

### What the oracle actually is

`task_success` is **case-insensitive substring containment** against the run's committed
answer — not an LLM judge, not exact match, not semantic similarity. For an all-digit
expectation, thousands separators are stripped from the answer first, so `1,000` satisfies
`1000` (that check once measured number formatting). This oracle shape has published prior
art: it is RAGAS's `StringPresence` — the deterministic tier every judge-free harness ends
up at — and naming it that is deliberate. What it is **not** is "faithfulness" in the
RAGAS/TruLens sense: those measure claim coverage of a whole answer with an LLM judge, and
our needle check is a single-fact evidence bound that stays deterministic. The two must
never be conflated, so we keep our own names.

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
eval-bench: scoring 32 live task(s) on [macos/aarch64 (8 cores) | ollama | gemma3:12b]
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

The suite spans fourteen **families**, each exercising a different part of the runtime. A
family is a bucket of tasks, and its gate is the floor mean over that bucket — so the **task
count is the denominator**, and the fraction beside each rate is the exact pass count. The
numbers are the committed `bench-v1` baselines (`macos/aarch64`, the two builds named
above), held to this table by `check-docs`.

| Family | Tasks | What a task proves | Ollama | llama.cpp |
| --- | ---: | --- | ---: | ---: |
| `tool` | 6 | The agent picks the right tool and its answer carries a fact only the tool could supply — including chaining one tool's output into the next call. | 1000 · 6/6 | 1000 · 6/6 |
| `react` | 3 | Whether to use a tool at all: an instruction naming a tool the run was never granted fires **nothing** (naming is not granting), a fact with no world-knowledge prior *is* looked up, and a question the model already knows is answered without reaching for anything. | 666 · 2/3 | 1000 · 3/3 |
| `script` | 3 | The agent runs a registered script in the sandbox and answers from what it computed. | 1000 · 3/3 | 666 · 2/3 |
| `reach` | 3 | How far the runtime reaches beyond the prompt: a [dataset](./datasets.md) it searches, a [memory](./memory.md) it recalls, and an app whose capability set is inherited rather than declared. | 1000 · 3/3 | 666 · 2/3 |
| `swarm` | 1 | N agents run in parallel and a gather merges their committed outputs — the answer must carry every agent's contribution. | 1000 · 1/1 | 1000 · 1/1 |
| `http` | 2 | A tool reached over the **network**, not a bundled subprocess: the runtime dials it over HTTP, presents a bearer credential resolved at dispatch, and pages through a result set whose answer is on the second page. | 0 · 0/2 | 0 · 0/2 |
| `failure` | 4 | Tools that error, hang, and return unusable payloads. The loop must surface the failure and let the model take another turn — and a healthy control fails if it starts distrusting every tool. | 750 · 3/4 | 750 · 3/4 |
| `menu` | 1 | Selection when the menu is as long as the runtime will present, rather than a choice between two obvious options. | 1000 · 1/1 | 1000 · 1/1 |
| `long` | 1 | The longest chain the runtime admits: six tool calls across four distinct tools, inside the eight-turn ceiling. | 0 · 0/1 | 0 · 0/1 |
| `adversarial` | 2 | Input that is trying to steer the agent — including an instruction planted in a **tool's output** — paired with a legitimate request that merely looks like one. | 500 · 1/2 | 1000 · 2/2 |
| `irrelevance` | 4 | Relevance detection, BFCL-style: two requests nothing on the granted menu can serve (an email send, a live weather read) where the correct move is to fire nothing and say so — beside two near-identically-phrased look-alikes a granted tool must serve, so an always-refuse policy fails the pair. | 1000 · 4/4 | 1000 · 4/4 |
| `memory` | 2 | LongMemEval-shaped, judge-free: a knowledge update whose superseded value stays live in the store (recall surfaces the conflict; the run must answer the NEW value), and an abstention when memory holds no answer. | 1000 · 2/2 | 500 · 1/2 |
| `scaffold` | 2 | Generated-app reach: the model plans and authors an entire project LIVE on this serve (one task per scheduled lane — contextual and codified), the app is then RUN, and the answer must carry an activation code that exists **only** inside the generated files — underivable unless the project actually reached the run's context. | 0 · 0/2 | 0 · 0/2 |
| `workflow` | 7 | A STORED workflow definition run by handle — canonical saved bytes, every warrant built server-side at run — through deterministic step kinds: a credentialed http dial, a three-way parallel quorum join, a typed conditional whose untaken arm commits a distinguished skip sentinel and provably never dials its endpoint (the high/low pair is scored as one property), a journal-backed 3-second timer that carries its parent's committed bytes, a flaky depot that answers only a FRESH retry identity, and a permanently-down branch under `continue` whose placeholder the join releases past. Every step is deterministic and every oracle token exists only on the harness fixture — the family measures the **runtime**, never the model. | 1000 · 7/7 | 1000 · 7/7 |

Each family reports its own gate (`task_success@swarm`) beside the suite-wide one, so a
regression in one capability is visible instead of being averaged away by the others.

### The failure family is a recovery rate {#recovery}

`task_success@failure` is, read precisely, a **tool-fault recovery rate**: every task in
the family injects a distinct fault class through a real MCP connector, and passing means
the loop surfaced the fault to the model and the model recovered — or, for the hang,
that the runtime's deadline dead-lettered honestly rather than inventing an answer.

| Fault class | Task | What the connector does |
| --- | --- | --- |
| error | `failure-tool-errors-recovers` | answers every call with a JSON-RPC error |
| garbage | `failure-garbage-recovers` | returns a truncated, unparseable payload |
| hang | `failure-timeout-deadletters` | sleeps far past the per-Mote tool deadline — the correct terminal is a **dead letter**, and a run that "answers" here has fabricated one |
| healthy control | `failure-control-healthy` | a working tool — the counterweight that fails if the loop starts distrusting every tool |

No shipping agent framework gates on fault injection today; the research context is
[Recovery-Bench](https://www.letta.com/blog/recovery-bench) and ReliabilityBench (research
context only — nothing here is a score on either). One honest caveat carried from that
work: recovery ability ranks orthogonally to raw capability, which is exactly why the
healthy control sits inside the family.

### The scaffold family measures the app the model builds {#scaffold}

`task_success@scaffold` drives the whole generated-app pipeline live: the model plans the
project, authors every file (a 20-minute scaffold budget — planning plus per-file decodes
on a 12B are minutes each; the suite's settle budget covers only the RUN that follows),
and the finished app is then run with a prompt that never mentions the code the answer
must contain. The activation code rides **only** the scaffold goal — never the run prompt,
never the stored envelope — so a passing answer is evidence the generated project itself
reached the run's context, and the canary-free fixture Apps beside it prove the code
cannot leak in from anywhere else.

**What a 0 means here is split by the `scaffold_completed@attempts` sentinel** — the
per-mille of scaffold attempts that reached `done` within budget. A gate of 0 with the
sentinel at 0 says the scaffolds never finished (the model ran out of its writing budget);
a gate of 0 with a non-zero sentinel says at least one project was fully written and run,
and the failure moved downstream into the run's grounding. The current captures show both
shapes: Ollama 0 with sentinel 0 (both scaffolds timed out mid-write), llama.cpp 0 with
sentinel 500 (the contextual project completed all seven files and ran — and the answer
still failed to ground in the project it had just written). Per-scaffold durations and
file counts are committed as ungated Spikes beside the gate.

**Hosted apps are excluded, by construction rather than choice.** The family scores an
app by RUNNING it and grading the run's committed answer; a hosted app has no blueprint
to run — its product is a served web process, not an answer a substring oracle can grade
— so the two scaffold tasks cover the two scheduled lanes (contextual and codified) and
say nothing about the hosted scaffold path.

### Reliability — pass^k {#reliability}

`pass_k4` re-runs three corpus-flagged **flagship** tasks four times each and scores a
task 1000 only when **all four** trials pass — tau2-style pass^k, on our own task set,
never leaderboard-comparable. Each trial runs on a fully fresh serve over a fresh state
dir; that is load-bearing, not hygiene. Run identity here derives from the bound recipe
and its args, so an identical re-invoke on the same serve **joins the committed run and
replays its result** — K same-serve "trials" would be one trial measured once and
reported K times. The harness therefore asserts each trial's journal is empty before
dispatch and that the four trials' instance ids are pairwise disjoint, and a model-free
CI test (`run_identity.rs`) pins that the detector reads differently under a secret
replay than under real re-execution.

The flagship set is corpus data (`"flagship": true` in `suite.json`), covered by the
suite digest — changing which tasks are flagship is a corpus change that voids the
baselines, never a quiet redefinition. Per-task pass^k values are recorded as ungated
Spikes beside the gated mean: a single K=4 binary draw on a marginal task flips
0-to-1000 between captures, and a gate whose noise exceeds any honest tolerance would
teach people to re-capture until it passed. The `pass_k4@trials` sentinel (1000 iff all
four trials executed) is what makes a skipped phase a hard, named regression — even for
a flagship whose captured value is 0 and could never catch it alone.

### Retrieval — Success@8 {#retrieval}

`retrieval_success_at_8` promotes what used to be a stderr-only preflight probe into a
ratcheted gate: ten queries, each with exactly one relevant document among the 61 in the
reach corpus, scored on whether that document ranks in the top 8. Published as exactly
what it is — **Success@k with binary single-relevant qrels, 61 documents, random floor
k/61 ≈ 131‰ per query** — and never as a BEIR-comparable Recall@k: the smallest BEIR
corpus is two orders of magnitude larger, and most of these queries deliberately target
the corpus's near-miss documents, which makes this a hard-negative discrimination gate
on the runtime's retriever rather than a corpus-scale retrieval score.

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

Beside the task latencies, a capturable run records three **RPC probe** distributions —
32 timed `StoreMemory`, `RecallMemory` and `QueryDataset` calls each, nearest-rank
p50/p95 — named for the exact RPC they time, because a memory recall and an ANN dataset
query have different cost profiles and a merged "retrieval latency" would hide which one
moved. The retrieval numbers honestly include query-embedding time: that is the cost a
task actually pays. All of them are Spikes — committed into the baseline **to be read**
(the README's Cost-and-latency table is checked against them), never compared by the
ratchet.

**Tokens.** The same telemetry attributes each task's summed model **output tokens**, so
the suite records tokens-per-task (per family and suite-wide) and tokens-per-success —
total output tokens per passed task, the cost of a success with the failures amortised
in, the same cost normalisation Gaia2 uses (calls + output tokens). Stated plainly: **no
input-token telemetry exists in this runtime** — the backend seam reports no input
count — so there is no input/cache split and no dollar cost here, ever; a metric whose
input the runtime does not record is not published. In OTel GenAI terms, the output
figure aggregates what `gen_ai.client.token.usage` (type `output`) would record per
operation; the task latencies have no semconv equivalent (they are agent-task
end-to-end figures, not per-model-call histograms), which is why these keep their own
names instead of borrowing OTel's.

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
- **K=4 is a small sample.** `pass_k4` at four trials separates "passes always" from
  "passes usually" and nothing finer; a per-task value is a single Bernoulli draw of
  p⁴, which is why only the mean gates and the per-task values stay recorded.
- **Ten queries is a probe.** `retrieval_success_at_8` moves in steps of 100‰ and its
  qrels are binary and single-relevant — a regression gate on this corpus, not a
  retrieval benchmark result.
- **Abstention is sentinel-shaped.** The irrelevance and memory-abstention oracles
  accept an exact refusal sentinel. The healthy look-alikes beside them are what stop
  an always-refuse policy from passing — but a subtler wrong refusal, phrased
  differently, is not measured.

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
