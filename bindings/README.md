# `bindings/` — language-binding workspaces

This directory holds **out-of-workspace** language bindings to kortecx core. Each
binding lives in its own subdirectory with its own Cargo workspace (or
language-native build system) and depends on kortecx's core crates from the
parent workspace.

## The load-bearing rule: **inward-only dependency**

```
┌─────────────────────────────────────────────────────────┐
│                    kortecx core                          │
│  (kx-mote, kx-journal, kx-projection, kx-executor, …)   │
│                                                          │
│            ▲                              ▲              │
│            │  depends on                  │  depends on  │
│            │                              │              │
│  ┌─────────┴────────┐         ┌──────────┴──────────┐   │
│  │ bindings/python  │         │ bindings/typescript │   │
│  │   (pyo3+maturin) │         │   (napi-rs)         │   │
│  └──────────────────┘         └─────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

**Bindings depend on core. Core NEVER depends on a binding and stays unaware
that Python or TypeScript exist.** This is non-negotiable.

### Why this rule matters

The rule keeps core's surface honest:

- **Core stays embeddable.** A Rust binary, a CLI, a TS binding, a Python
  binding, or a future Go binding all consume the same `kx-*` API. Adding
  Python-specific affordances inside core (e.g., a `#[pyclass]` on
  `MoteDef`) would couple core to PyO3's release cadence and ABI.
- **Core stays cross-compilable.** PyO3 requires Python headers + the right
  interpreter ABI; napi-rs requires Node ABI. Pulling those into core means
  every cross-compile target needs them — including server, CLI, embedded
  builds where Python/Node are absent.
- **Core stays one trust boundary.** Every binding adds an unsafe-ish bridge
  (PyO3 GIL invariants, napi-rs's Promise<>JsObject lifetimes, etc.). Each
  bridge belongs in the binding crate, audited in isolation.
- **Core stays semver-clean.** A Python-side change (new TypedDict shape, new
  binding function) doesn't touch core's release. Each binding versions
  independently.

### What this means in practice

- The core workspace (`Cargo.toml` at the repo root) **excludes** every
  `bindings/*` subdirectory. They are NOT workspace members; they don't
  affect `Cargo.lock` resolution.
- Each binding has its own `Cargo.toml` / `package.json` / `pyproject.toml`
  declaring kortecx core as a **`path = "../../"`** dependency (during dev)
  or as a published crate (post-`cargo publish`).
- Core code (every `crates/kx-*/src/**.rs`) must compile and pass tests with
  NO `bindings/*` files present. Verifiable: `git rm -rf bindings/` then
  `cargo test --workspace` still passes.
- Core's `Cargo.toml` MUST NEVER list `pyo3`, `maturin`, `napi-rs`, or any
  binding-specific dep in `[workspace.dependencies]`. Those live exclusively
  in the per-binding workspace.

### How a binding is added (the high-level process — NOT yet wired)

This document describes the rule. Implementing a binding is its own PR. The
shape:

1. `cd bindings/python` (or `bindings/typescript`).
2. Create the binding crate's `Cargo.toml` (for Python: pyo3-extension-module
   library; for TS: cdylib that napi-rs wraps).
3. Add path-deps on the core crates needed (`kx-mote`, `kx-executor`, etc.).
4. Define the binding surface — typically a thin wrapper that exposes a
   handful of host-language idiomatic types.
5. Configure the language-native build (`maturin develop` for Python;
   `napi build` for TS).
6. Add a separate CI job that builds the binding + runs language-native
   tests against it.

## Current state

**Both `bindings/python/` and `bindings/typescript/` are STRUCTURE-ONLY** at
the time of this commit. They contain a `README.md` (the inward-only rule
restated for that specific binding) and no source files. Implementing each
binding is a separate, scoped piece of work that lands AFTER the kortecx
runtime promise lands at P1.9 (kx-executor) — there's nothing meaningful to
bind from a language-host perspective until the runtime is fully assembled.

## See also

- The workspace root `Cargo.toml` — `[workspace].exclude` includes
  `bindings/python` and `bindings/typescript`.
- `bindings/python/README.md` — Python-specific application of the rule.
- `bindings/typescript/README.md` — TypeScript-specific application.
