# kortecx

> The distributed runtime for AI agents.
> **Knowledge → Intelligence.**

[![CI](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml/badge.svg)](https://github.com/Kortecx/kortecx/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

---

## Build agents that ship

AI agents work in the demo and fall over in production. The gap is **reliable orchestration** —
dispatching tasks across workers, retrying on failure, guaranteeing delivery, surviving the
messy reality of model calls and external APIs.

kortecx closes that gap. It's the execution engine beneath your agents — the layer that turns
clever scripts into production software you can trust with real work.

Not a model. Not a database. Not a chat app. The execution kernel beneath them.

## Innovate at pace

Building agents shouldn't be a fight with infrastructure. kortecx is designed so you can move
fast without leaving correctness behind.

- **Start in minutes.** One workspace, one command, and you have a working runtime. No platform
  setup, no operator-grade configuration before you can experiment.
- **Run anywhere.** Laptop today, cluster tomorrow, hosted at scale when you need it — same
  runtime, same APIs, same guarantees.
- **Iterate without fear.** Every agent step is a durable record. Crash mid-workflow and resume
  cleanly. Change your mind and re-run with full traceability.
- **Observable by default.** No invisible agent loops. No hidden state. Nothing to babysit.

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
