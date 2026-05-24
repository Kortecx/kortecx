# kortecx

> The distributed runtime for AI agents.
> **Knowledge → Intelligence.**

🌐 **[kortecx.com](https://kortecx.com)** &nbsp;·&nbsp; built in the open at [Kortecx/kortecx](https://github.com/Kortecx/kortecx)

[![CI](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml)
[![Private-content leak check](https://github.com/Kortecx/kortecx/actions/workflows/leak-check.yml/badge.svg?branch=main)](https://github.com/Kortecx/kortecx/actions/workflows/leak-check.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![MSRV](https://img.shields.io/badge/MSRV-1.94.0-orange.svg)](rust-toolchain.toml)
[![Rust Edition](https://img.shields.io/badge/Rust-2021-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/index.html)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-blue.svg)](#)
[![Status](https://img.shields.io/badge/status-pre--alpha-yellow.svg)](#whats-in-this-repo-today)

---

## The gap we're closing

AI agents work in the demo and fall over in production. The gap is **reliable orchestration** —
dispatching tasks across workers, retrying on failure, guaranteeing delivery, surviving the
messy reality of model calls and external APIs.

kortecx is being built to close that gap: an execution kernel that turns clever agent scripts
into production software you can trust with real work.

Not a model. Not a database. Not a chat app. A runtime — the execution kernel beneath them.

## What kortecx will guarantee (the goal)

These are the guarantees the runtime is being built around. They are the design contract, not
a description of what runs today (see the next section for that).

- **Exactly-once orchestration of non-deterministic, world-mutating steps under failure.** When
  an agent calls a model or mutates the world, the runtime ensures the step's outcome is a
  durable fact that downstream work reads — never a recomputation that drifts.
- **Replay-from-journal recovery.** If the process dies mid-workflow, restart re-reads what
  committed steps did rather than re-running them. A side-effecting step is never re-effected.
- **One runtime, three deployment shapes.** Same APIs and same guarantees from laptop to
  cluster to hosted.
- **Observable by design.** Every step a durable record. No invisible agent loops.

## What's in this repo today

kortecx is in early development. The public repo currently ships the **foundations**, not the
full runtime:

- **`kx-mote`** — pure types for the atomic execution unit: Mote identity, lifecycle state
  machine, three-way non-determinism tag.
- **`kx-content`** — content-addressed payload storage (BLAKE3) with atomic-per-object writes;
  local-FS and in-memory backends behind a single trait.
- **`kx-journal`** — append-only journal (SQLite-backed, `BEGIN IMMEDIATE`, dedupe-by-key,
  WAL + `synchronous=FULL`) plus an in-memory backend for fixtures.
- **`kx-projection`** — pure read-side fold over the journal; cycle-tolerant BFS traversal;
  snapshot isolation for the scheduler.
- **`kx-llamacpp-sys`** + **`kx-llamacpp`** — the in-process llama.cpp safe wrapper (RAII,
  lifetime-tied; no unsafe outside the FFI boundary). Generate, embed, chat: three calls.
- Reproducible, byte-deterministic release builds, verified in CI on every push.
- Structured tracing wired across the workspace.

### Testing discipline

Every foundation crate is gated by the same minimum bar before it can be built upon:

- **Functionality + determinism** — every API has unit + integration tests; every operation
  that promises determinism asserts it (run twice, identical output).
- **Property tests** (`proptest`) — every function that accepts user-supplied input has at
  least one property pinned across the input space, not just hand-picked cases.
- **Concurrency tests** — every type that claims `Send` is exercised under ≥ 2 real threads.
- **Doctests** — every non-trivial public method has a runnable doctest.
- **Real-GGUF end-to-end smoke** — the inference wrapper's full pipeline (load → tokenize →
  decode → sample → detokenize → KV-cache save/restore) is exercised against a real model in
  CI on every push.
- **Cross-platform** — Linux x86_64 in GitHub Actions CI; macOS Apple Silicon (Metal) verified
  locally on every PR.

The journal, executor, scheduler, capability broker, inference router, and runtime binary are
**under active development** in the open. Until they land, `cargo build` produces foundational
types — not yet a runnable agent runtime. The roadmap is to land them one crate at a time,
each gated on the contract above.

## Installation

Add kortecx to your Rust project:

```toml
[dependencies]
kortecx = { git = "https://github.com/Kortecx/kortecx" }
```

Then use the standard Rust workflow:

```bash
cargo build
cargo test
```

Rust 1.94.0+ is required.

### Or build from source

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
cargo build --workspace
cargo test --workspace
```

## Contributing

Contributions are welcome. Please open an issue to discuss substantial changes before sending
a pull request.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

## Links

- **Website:** [kortecx.com](https://kortecx.com)
- **Issues:** [github.com/Kortecx/kortecx/issues](https://github.com/Kortecx/kortecx/issues)
- **CI:** [Actions tab](https://github.com/Kortecx/kortecx/actions) — both `just ci` and the
  real-GGUF smoke job must pass on every PR.
