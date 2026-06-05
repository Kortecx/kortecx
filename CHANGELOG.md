# Changelog

All notable changes to kortecx are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). kortecx is in early
development; interfaces may change before 1.0 — pin a commit if you build on it.

## [Unreleased]

## [0.1.0] — the reachable runtime

The first release where the durable runtime is **reachable end to end**: a server,
a CLI, recipes, an audit trail, and a live event stream, on top of the
exactly-once durability spine.

### Added

- **`kx` CLI** — one FFI-free binary (`crates/kx-cli`). `run`/`replay`/`digest`
  drive the engine locally; `serve` hosts the gateway; `invoke`/`submit`/
  `projection`/`content`/`events`/`signatures` are gRPC clients of a running
  gateway. Agent-ergonomic `--wait` runs the runtime like a function and returns
  one committed result; `--json` everywhere; a typed exit-code contract
  (`0` ok / `2` usage / `3` wait-timeout-resumable / `1` rpc+io).
- **Gateway server** — `kx serve` hosts the `KxGateway` gRPC service over an
  embedded coordinator + local worker (`crates/kx-gateway`, `crates/kx-gateway-core`).
  Bearer-token auth with **deny-all default** and **server-derived identity**;
  `--dev-allow-local` for loopback development.
- **Inbound recipe execution** — `Invoke` binds a published recipe by handle to
  JSON args and runs it to a committed terminal Mote, exactly-once
  (`crates/kx-invoke`).
- **Recipe library + prompt templating** — five reusable, deterministic recipes
  (`map_reduce`, `fan_out_gather`, `retry_until_critic`, `react_tool_loop`,
  `image_batch_describe_reduce`) and a pure, fail-closed prompt-template engine
  (`crates/kx-workflow`).
- **Audit trail** — an off-truth-path, best-effort JSONL audit sink that records
  the run lifecycle without ever touching the projection digest
  (`crates/kx-audit`); enabled with `kx run --audit-log <path>`.
- **Live event stream** — `StreamEvents` is a true resumable live tail, with a
  WebSocket bridge; `kx events --follow` consumes it and auto-resumes.
- **Durable catalog & fleets** — a sharable signature/recipe catalog with durable
  SQLite-backed ledgers (`crates/kx-catalog`) and team/fleet membership
  (`crates/kx-fleet`).
- **Tiered install automation** — `just setup` (FFI-free), `just setup-inference`
  (opt-in native backend), `just fetch-demo-model` (SHA-256-verified GGUF), a
  tiered `just doctor` with per-OS install hints, and `just verify-quickstart`
  (a docs-as-test gate that runs the README quickstart and asserts the canonical
  digest).
- **Documentation** — a production-grade README (quick start → serve → inspect),
  refreshed `GLOSSARY.md`, and this changelog.

### Guarantees (carried from the durability spine)

- A world-mutating step takes effect **exactly once** across crashes, retries, and
  redistribution.
- All live state is a **pure fold** of an append-only journal; recovery re-folds
  the log. Cold re-fold of a 25k-Mote journal stays sub-linear (gated in CI).
- The `kx` binary installs with **Rust only** — no C++ toolchain (proven by a
  dependency-wall test and an FFI-free CI build job). llama.cpp is opt-in for local
  inference.

### Known limitations

Plaintext gRPC (front with TLS for non-loopback); bearer-token auth with no
multi-tenant isolation yet; single-system journal writer; single-stream inference
with model-by-path (no registry); audit-log + event-stream observability (no
metrics/OTel export yet). See the README's *Production notes & known limitations*.

[Unreleased]: https://github.com/Kortecx/kortecx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Kortecx/kortecx/releases/tag/v0.1.0
