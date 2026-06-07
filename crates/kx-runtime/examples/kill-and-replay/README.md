# kill-and-replay — the P1 exit-gate proof

This directory is the reproducible artifact for the novel kortecx claim:

> **A committed non-deterministic, world-mutating step is a fact; recovery
> re-reads what it did, never re-runs it.**

Ray and Temporal reschedule work across workers on failure — that is not the
novel claim. The novel claim is provable on a single node, and that is what this
proves.

## The demo workflow

`kx-runtime` drives a small but non-trivial Mote DAG (`src/workflow.rs`):

```
M1   PURE root (deterministic compute)
 ├─> S    READ-ONLY-NONDET topology shaper  ──commits a TopologyDecision──>  W0, W1 (PURE workers)
 ├─> M2   READ-ONLY-NONDET (model sample)
 ├─> Wstc WORLD-MUTATING  StageThenCommit       [scenario-1 crash target]
 └─> M3   WORLD-MUTATING  ValidateThenCommit ──critic──> M3c PURE  [scenario-2 crash target]
```

Eight Motes commit in total: the six declared above plus the two workers the
shaper materializes at runtime. Every body executes **deterministically**
(`TestMoteExecutor::deterministic` + a deterministic broker), so the journal —
and the projection folded from it — is byte-identical across runs, processes,
and machines. That determinism is the precondition for the proof.

## Reproduce on a fresh checkout

The kill-restart-assert cycle is a test (`tests/kill_and_replay.rs`):

```sh
cargo test -p kx-runtime --test kill_and_replay
```

It spawns the real `kx-runtime` binary, kills it with a hard `SIGABRT`
(`std::process::abort`) at a precise window over an on-disk SQLite journal, then
restarts a **fresh process** that recovers by replaying the journal. Two
scenarios run:

- **`pre-commit-stc`** — kill mid `StageThenCommit` (after `EffectStaged` + the
  broker stage, before `Committed`). Recovery re-dispatches; the deterministic
  idempotency key dedups the external effect → exactly-once.
- **`post-commit-vtc`** — kill the instant the `ValidateThenCommit` Mote's
  `Committed` is durable. Recovery **re-reads** the committed `result_ref`,
  never re-running the effect — the headline claim.

Or drive the binary by hand:

```sh
J=/tmp/kx.sqlite C=/tmp/kx-content
kx-runtime run    --journal "$J" --content "$C"                      # clean run  -> 8/8
kx-runtime run    --journal "$J" --content "$C" --crash-at post-commit-vtc   # aborts (SIGABRT)
kx-runtime replay --journal "$J" --content "$C"                      # recovers   -> 8/8
kx-runtime digest --journal "$J" --content "$C"                      # projection digest
```

## What the bytes mean — and what proves the claim

The journal is an **append-only log of committed facts** (SQLite,
`synchronous=FULL` + WAL). The graph state is a *projection* folded from that
log; it is never the durable truth. Each entry is small and fixed-size
(`journal-entry.md` §8 caps an entry at ≤4304 bytes); large payloads live in the
content store, referenced by 32-byte BLAKE3 `ContentRef`s.

The proof is a **projection digest**: BLAKE3 over the committed-result set
(`mote_id ‖ result_ref ‖ nd_class` for every committed, non-repudiated Mote, in
`MoteId` order). The canonical value for this workflow is in
[`reference-digest.txt`](./reference-digest.txt):

```
7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9
```

The exit gate is three assertions, all checked by the test:

- **(a)** the committed-result set after a crash-and-replay is **bit-identical**
  to a clean run (same digest);
- **(b)** **no Mote has more than one `Committed` entry** — exactly-once;
- **(c)** a **fresh process** that has only the journal file folds it to a
  **bit-identical projection** (the same digest) — i.e. recovery re-read what the
  steps did rather than re-running them, and it does so identically on a
  different machine.

Because the digest is over committed *facts* (not the SQLite container's bytes,
which are not byte-stable across platforms), the digest — not a checked-in
`.sqlite` blob — is the canonical, portable invariant a skeptic verifies:

```sh
test "$(kx-runtime digest --journal "$J" --content "$C")" = "$(cat reference-digest.txt)"
```

Topology survives replay too: the shaper `S` commits a `TopologyDecision`, and on
every replay the materializer re-derives `W0`/`W1` **deterministically from the
committed decision** — it never re-runs the shaper. That is the P1 exit gate's
"handles a runtime-discovered topology decision across that replay."
