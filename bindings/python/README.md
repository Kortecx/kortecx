# `bindings/python` — Python language binding (PyO3 + maturin)

**Status**: structure-only. No source files yet. The Python binding is a
separate piece of work scheduled to land AFTER `kx-executor` (P1.9) makes a
runtime promise meaningful to bind from a host language.

## The rule (restated for Python)

**Bindings depend on core. Core NEVER depends on this binding and stays
unaware that Python exists.**

What this means concretely:

- **No `pyo3` in any `kx-*` core crate's `Cargo.toml`.** The pyo3 dep lives
  exclusively in `bindings/python/Cargo.toml`.
- **No `#[pyclass]` / `#[pyfunction]` / `#[pyo3(...)]` attributes on any
  type defined in a `kx-*` core crate.** Wrapper types in
  `bindings/python/src/` re-expose core types with PyO3 attributes.
- **No "Python is special" carve-outs in core.** If Python needs a different
  serialization shape, that's the binding's transform, not core's
  responsibility. (Bincode wire format is canonical; binding can map to
  Python dicts / dataclasses on its side.)
- **No PyO3 features in the workspace `Cargo.toml`'s
  `[workspace.dependencies]`** for any core crate to use.

## Why PyO3 + maturin

When the Python binding lands:

- **[pyo3](https://pyo3.rs/)** — the Rust → Python bridge. ABI3-stable
  builds (one wheel works across CPython 3.8+).
- **[maturin](https://www.maturin.rs/)** — the build tool that wraps cargo
  to produce a Python wheel. Replaces the older `setuptools-rust` path.
- **PEP 517 / PEP 518** via maturin's `pyproject.toml` shape.

## Expected directory shape (NOT yet present)

```
bindings/python/
├── README.md            ← this file
├── Cargo.toml           ← out-of-workspace; lib.crate-type = ["cdylib"]
├── pyproject.toml       ← maturin config
├── src/
│   └── lib.rs           ← #[pymodule] entrypoint + per-host wrapper types
└── tests/
    └── test_smoke.py    ← runs against the built wheel
```

## The binding's responsibility surface

The Python wrapper is a **thin transform layer**, not a feature layer. Its
job is to:

1. Re-export core types with idiomatic Python shapes (`MoteId` as a Python
   `int`-like class, `JournalEntry` as a Python dataclass, etc.).
2. Translate Rust `Result<T, E>` to Python exceptions via PyO3's
   `PyErr` conversion.
3. Convert between Python objects (bytes, dicts, strings) and the bincode
   canonical shape core uses internally.
4. Honor the GIL: every public Python-callable function takes / releases
   the GIL correctly. Long-running core calls (inference, journal fold)
   release the GIL via `Python::allow_threads`.

Things the wrapper does NOT do:

- It does NOT add features to core. Bindings reflect; they don't extend.
- It does NOT define new domain types. A new domain concept goes in core
  first; the wrapper picks it up after.
- It does NOT cache state. Core owns state; the wrapper holds handles.

## CI

When the binding lands, it gets its own CI job (separate from the core
workspace's `ci.yml`):

- Builds the wheel via `maturin build --release` on the matrix of
  (CPython 3.8, 3.9, 3.10, 3.11, 3.12) × (Linux x86_64, Linux aarch64,
  macOS x86_64, macOS aarch64).
- Runs `pytest bindings/python/tests/` against each wheel.

The job is **independent** of the core CI workflow's `test` job — the
binding's tests CAN run in parallel with core's tests because the
inward-only dependency is preserved.

## Audit invariant — verifiable by command

The inward-only rule is verifiable mechanically. From the workspace root:

```bash
# A: core crates' Cargo.toml MUST NOT mention pyo3 / maturin.
grep -rE "pyo3|maturin" kx-*/Cargo.toml && echo "RULE VIOLATED" || echo "ok"

# B: core code MUST NOT contain #[pyclass] / #[pyfunction] / #[pymethods] /
#    #[pymodule] / #[pyo3] attributes anywhere in src/.
grep -rE "#\[py(class|function|methods|module|o3)" kx-*/src/ && echo "RULE VIOLATED" || echo "ok"

# C: `cargo test --workspace` MUST pass with bindings/ removed.
mv bindings /tmp/bindings.bak && cargo test --workspace && mv /tmp/bindings.bak bindings
```

A future CI job can codify (A) + (B) as an audit step.
