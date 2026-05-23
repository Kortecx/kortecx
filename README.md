# kortecx

> The distributed runtime for AI agents.
> **Knowledge → Intelligence.**

[![CI](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml/badge.svg)](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

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
runtime:

- The workspace, build system, and CI gates that the runtime crates land into.
- `kx-mote`, the pure-types crate defining the atomic execution unit (Mote types, identity,
  lifecycle state machine, three-way non-determinism tag).
- Reproducible, byte-deterministic release builds (verified in CI on every push).
- Structured tracing wired across the workspace.

The journal, executor, scheduler, capability broker, inference router, and runtime binary are
**under active development** in the open. Until they land, `cargo build` produces foundational
types — not yet a runnable agent runtime. The roadmap is to land them one crate at a time, each
gated on the contract above.

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
