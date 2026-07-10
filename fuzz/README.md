# Fuzzing the untrusted-byte parse paths

kortecx loads third-party models and runs third-party tools, so the byte parsers that accept **untrusted
input** are the highest-value review + fuzz surface. This is a [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz)
(libFuzzer) harness for the FFI-free, fail-closed parsers.

## Targets

| target | function | untrusted input |
|---|---|---|
| `decode_plan` | `kx_planner::decode_plan` | a model's PROPOSED plan bytes (the SN-8 fence) |
| `decode_entry` | `kx_journal::decode_entry` | a journal-entry record (the synchronization substrate) |
| `fold_checkpoint` | `kx_projection::FoldCheckpoint::from_bytes` | a projection checkpoint a recovering runtime folds |

Each function is documented **total + panic-free over arbitrary bytes** with DoS caps; the fuzzer asserts
that — a panic / OOM / hang is a finding.

## Run it

Fuzzing needs the **nightly** toolchain (sanitizer + coverage instrumentation). The repo pins stable via
`rust-toolchain.toml`, so force nightly:

```sh
cargo install cargo-fuzz              # once
RUSTUP_TOOLCHAIN=nightly cargo fuzz build
RUSTUP_TOOLCHAIN=nightly cargo fuzz run decode_plan            # runs until a crash / Ctrl-C
RUSTUP_TOOLCHAIN=nightly cargo fuzz run decode_plan -- -max_total_time=60   # bounded
```

CI runs a short **fuzz-smoke** (~45 s/target) on every PR to catch a newly-introduced crash; deep/long
fuzzing (hours, a persisted corpus) is a separate scheduled run. Smoke baseline at introduction:
~10.7 M total executions across the three targets, **0 crashes**.

## Not yet covered (follow-ups, tracked honestly)

- **The FFI parse paths** — the llama.cpp GGUF loader (`kx_llamacpp::Model::load`) and the vendored
  image/`mmproj` decoders (`kx_llamacpp::mtmd`). Highest value (they run *below* the Rust safety model),
  but they need the C++ toolchain + the vendored submodule, so they build in a separate FFI-enabled fuzz
  profile — not in this FFI-free smoke.
- **`kx_toolcall::parse_tool_call`** — parses untrusted model tool-call output, but takes a `WarrantSpec`
  (it early-returns with no grants); a target needs a fixed non-empty warrant fixture.
- **`kx_content` rkyv zero-copy access** — currently exercised only in a unit test; add a target when a
  production `rkyv::access` call site lands.

Corpora + crash artifacts under `fuzz/corpus/` and `fuzz/artifacts/` are git-ignored.
