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
[![Status](https://img.shields.io/badge/status-early%20development-yellow.svg)](#where-we-are)

---

## The gap we're closing

AI agents work in the demo and fall over in production. The gap is **reliable orchestration** —
dispatching tasks across workers, retrying on failure, surviving crashes, and never re-running a
step that already touched the outside world.

kortecx is an execution kernel that turns clever agent scripts into production software you can
trust with real work. Not a model. Not a database. Not a chat app. A runtime — the execution
kernel beneath them.

## What makes it different

Most tools help you *write* an agent. The hard part was never writing it — it's trusting it to
run unattended, at scale, against the real world, and to recover cleanly when a step fails partway.
That's an infrastructure problem, and it's the one keeping agents stuck in demos. It's the problem
kortecx exists to solve.

The division of labor is clean: **the model plans and manages, Motes take the action, your data
comes from wherever it lives, and the runtime captures everything and guarantees it runs reliably.**

Agents are non-deterministic and they act on the world — they call models, hit APIs, move money,
change state. Run that at scale and the failure modes compound: a retry double-charges a customer,
a crash loses half a job, a redistributed task silently runs twice. Today every team rebuilds the
same fragile glue to cope, and most never get past the demo. kortecx makes reliability a property
of the runtime instead of something each team reinvents:

- **Trustable.** A step that touches the world takes effect exactly once. Crashes, retries, and
  redistribution never double-apply or silently drop work — so you can hand an agent real
  responsibility and step away.
- **Scales without a rewrite.** The same workflow runs on a laptop or across a fleet, with the
  same guarantees. Scale is a deployment choice, not a re-architecture.
- **Always delivers.** Workflows survive process death and pick up exactly where they left off,
  resuming committed work rather than starting over or stalling.
- **No infrastructure tax.** Durability, recovery, and coordination are built in, so teams ship
  agents instead of the plumbing beneath them.
- **Bring your own data + tools.** Read from any store, call any API or MCP tool — the runtime
  records what happened, captures the result, and serves it durably. You own data legitimacy; we
  own the durability.
- **Reusable, not rebuilt.** Actions, workflows, tools, and context live in a sharable catalog:
  publish a guaranteed action once, reuse it with new parameters, share it with your team.

This is the missing layer between a clever agent script and production software — the foundation
that makes putting agents in charge of real, consequential work a reasonable thing to do, and
turns agentic automation from a demo into something organizations can adopt at scale.

## Correctness

The core guarantee — a step that changes the world takes effect exactly once, even across crashes
and retries — is something you can verify for yourself. The [Try it](#try-it) demo below crashes a
workflow mid-run and shows it recover to an identical result.

Beyond that, every change to the project is gated by an automated test suite that exercises the
runtime under simulated failures, including crashes during live workflows, and checks that no work
is ever lost, duplicated, or double-applied. The suite runs on every commit, and a change cannot
merge until it passes.

## Try it

The runtime ships a small binary that demonstrates the core guarantee directly. Build the
workspace, then drive the canonical workflow, crash it mid-commit, and replay:

```bash
# 1. Run the demo workflow to completion, capturing its deterministic digest.
cargo run -p kx-runtime -- run    --journal /tmp/kx.db --content /tmp/kx-content
#    -> <digest> (N/N committed)

# 2. Start fresh, but hard-abort right after a side effect commits.
rm -f /tmp/kx.db; rm -rf /tmp/kx-content
cargo run -p kx-runtime -- run    --journal /tmp/kx.db --content /tmp/kx-content --crash-at post-commit-vtc

# 3. Recover from the journal and finish the run.
cargo run -p kx-runtime -- replay --journal /tmp/kx.db --content /tmp/kx-content
#    -> same <digest> — the crashed step was re-read, not re-run.
```

Same digest across the clean run and the crash-then-replay run is the exactly-once property,
demonstrated end to end.

Want to see inference on its own? Point the examples at any GGUF model:

```bash
cargo run -p kx-llamacpp --example generate -- /path/to/model.gguf "Once upon a time"
cargo run -p kx-llamacpp --example chat     -- /path/to/model.gguf
cargo run -p kx-llamacpp --example embed    -- /path/to/model.gguf "embed this"
```

## Where we are

kortecx is in **early development**, built in the open.

Today it offers a working runtime core: agent steps run with exactly-once guarantees and survive
crashes by replaying from the journal — on a single node or distributed across workers.

We're headed toward stable public APIs, higher-level authoring surfaces, and a hosted platform.
The internals are real and tested, but interfaces will change before 1.0 — pin a commit if you
build on it now.

## Build from source

```bash
git clone https://github.com/Kortecx/kortecx.git
cd kortecx
cargo build --workspace
cargo test  --workspace
```

Rust **1.94.0+** is required. The full pre-merge gate (format, clippy `-D warnings`, workspace
tests, dependency audit, docs, FFI link, byte-determinism, and the real-GGUF inference smoke) runs
on Linux x86_64 in CI; macOS Apple Silicon (Metal) is verified locally on every change.

## Contributing

Contributions are welcome. Please open an issue to discuss substantial changes before sending a
pull request.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

## Links

- **Website:** [kortecx.com](https://kortecx.com)
- **Issues:** [github.com/Kortecx/kortecx/issues](https://github.com/Kortecx/kortecx/issues)
- **CI:** [Actions tab](https://github.com/Kortecx/kortecx/actions) — both the full gate and the
  real-GGUF smoke job must pass on every PR.
</content>
</invoke>
