# `kx-llamacpp-sys` — llama.cpp submodule pin

The `llama.cpp/` directory in this crate is a **git submodule** pinned to a
specific upstream commit. The submodule SHA stored in this repository's tree
is the **load-bearing pin** — `cargo build` rebuilds the FFI bindings against
whatever commit the submodule points to, so an unaudited submodule advance
silently changes the FFI surface every binary in this workspace links
against.

## Current pin

| Field | Value |
|---|---|
| Submodule path | `crates/kx-llamacpp-sys/llama.cpp` |
| Upstream URL | `https://github.com/ggerganov/llama.cpp` |
| Tracking branch | `master` (advisory only — the SHA is what builds) |
| **Pinned commit** | `d2462f8f7ac6d80070a587ffebf6cd73730f4280` |
| **Version-tag-derived name** | `b9310-280-gd2462f8f7` (280 commits past build tag `b9310`) |
| Date of pin entry | 2026-06-16 |

The pinned commit subject is `chat: fix LFM2/LFM2.5 ignoring json_schema (#24377)`.

## Why this matters

The FFI surface of llama.cpp is **not stable**. Upstream regularly:

1. **Renames `llama_*` functions** (e.g., `llama_get_logits` → `llama_get_logits_ith`).
2. **Changes struct layouts** (`llama_context_params` adds/removes/reorders fields).
3. **Reorders enum variants** (`llama_pooling_type` discriminants may shift).
4. **Changes ownership semantics** (a function that returned an owned buffer may begin returning a borrow, or vice versa).

Any of these surfaces as one of:

- A build failure (best case: bindgen can't generate the binding).
- A silent ABI mismatch (worst case: same Rust signature, different C semantics — memory corruption under specific inputs).

This is why the peer-review of P1.7-foundation flagged `kx-llamacpp-sys` as
*ASSUMED-with-tail-risk* (the link compiles, smoke tests pass, but FFI bugs
hide until specific call patterns hit them).

> **Note (agent-model integration).** This pin supports both the Qwen3 agent
> stand-in (`qwen3` arch, the `llama_flash_attn_type` enum on
> `llama_context_params.flash_attn_type`, `type_k`/`type_v` KV-cache element
> types) **and the Gemma-4-12B omni model** — the 2026-06-16 bump's motivation:
> the `gemma4` text arch (`src/llama-arch.cpp`) plus the `gemma4uv` *unified
> vision* projector (`tools/mtmd/models/gemma4uv.cpp`), which the prior
> 2026-05-25 pin did not recognize (`clip_init: unknown projector type:
> gemma4uv`). GGUF stays format V3, so existing Qwen3 fixtures load unchanged.

The pin makes ABI drift **an explicit, audited event** rather than a
shadow upgrade.

## Upgrade procedure (the audit ritual)

**Do not** advance the submodule without running this checklist. The cost of
a single ABI-drift bug in production is significantly higher than the cost
of this audit.

### 1. Capture the prospective new pin

```bash
cd crates/kx-llamacpp-sys/llama.cpp
git fetch origin master
git log --oneline HEAD..origin/master | wc -l   # commits since current pin
git log --oneline HEAD..origin/master | head -20  # eyeball the recent commit subjects
NEW_SHA=$(git rev-parse origin/master)
NEW_NAME=$(git describe --tags origin/master)
```

### 2. Diff the FFI surface

```bash
git -C crates/kx-llamacpp-sys/llama.cpp diff HEAD origin/master -- 'include/llama.h' 'ggml/include/'
```

Reading the header diff is the load-bearing audit step. Look for:

- **Removed or renamed functions** referenced by `kx-llamacpp/src/*.rs` — these become Rust build failures (the import vanishes from bindings.rs).
- **Reordered or resized struct fields** in any `llama_*_params` or `llama_*_context` type — Rust's struct layout will mismatch C's.
- **Reordered enum variants** — `#[repr(i32)]` discriminants in `kx-llamacpp/src/*.rs` may quietly point at the wrong variant.
- **Changed pointer ownership semantics** in function signatures — a `const llama_*` becoming `llama_*` (or vice versa), or a function that "returns a borrowed pointer" becoming "returns an owned pointer that the caller must free."

### 3. Build + run the full workspace test suite

```bash
git -C crates/kx-llamacpp-sys/llama.cpp checkout $NEW_SHA
cargo build --workspace 2>&1 | tee /tmp/build.log
cargo test --workspace 2>&1 | tee /tmp/test.log
cargo test -p kx-llamacpp --test smoke
cargo test -p kx-llamacpp --test stress
cargo test -p kx-llamacpp --test concurrency
```

A clean build + green tests is **necessary but not sufficient** — the smoke
tests do not cover every FFI call pattern. Inspect the build.log and test.log
for new warnings, deprecation notices, or `[[deprecated]]` annotations on
functions the wrapper uses.

### 4. Run the safe-wrapper unsafe-block audit

For each file in `kx-llamacpp/src/`, re-read every `unsafe { ... }` block
against the new `llama.h` header. Verify:

- The FFI function still exists with the same name and signature.
- Pointer ownership semantics (in, out, in-out, owned, borrowed) match what the wrapper assumes.
- `Drop` impls call the correct `*_free` function.
- Lifetime bounds (`'b: 'm` on `Model<'b>` / `Context<'m, 'b>`) still match the actual ownership chain in the FFI.

### 5. Update this document

When the audit passes:

```bash
# Commit the submodule advance in the parent repo
git add crates/kx-llamacpp-sys/llama.cpp
git commit -m "kx-llamacpp-sys: advance llama.cpp pin to $NEW_NAME ($NEW_SHA)"
```

Then edit this file's `## Current pin` table to record the new SHA, the new
`git describe` name, the new pin date, and a one-line note on the most
load-bearing change observed in the audit (if any).

### 6. Cross-platform smoke

The pin advance MUST be exercised on **both supported platforms** before merge:

- **Apple Silicon (darwin-arm64)**: locally — `cargo test --workspace`.
- **Linux (`just ci`)**: the OSS Actions workflow at `.github/workflows/`.

Apple Silicon catches Metal-backend regressions (the `ggml-metal` static
archive); Linux catches the CPU-only path. Both must pass.

## What this document does NOT cover

- **CUDA / GPU upgrades.** The build pins `GGML_CUDA=OFF` per D28 (cloud-side serving uses vLLM / SGLang). When P5 brings in a CUDA backend, that's a separate audit on a separate pin.
- **GGUF format migrations.** llama.cpp occasionally bumps GGUF major versions (e.g., GGUF v2 → v3). The pinned SHA implicitly determines which GGUF major version the wrapper can load. Model files in the test fixtures may need rebuilding after a pin advance that crosses a GGUF major.
- **Sandboxed inference (D41).** When the runtime gains the `MacOsSandbox` / `Bwrap` executors at PR 9, llama.cpp's `mmap`-vs-`read`-from-disk behavior under a restricted FS namespace may need additional audit. Out of scope for this document.

## Pin history

| Date | SHA | `git describe` | One-line audit note |
|---|---|---|---|
| 2026-05-25 | `1a03cf47f67b...` | `gguf-v0.18.0-826-g1a03cf47f` | Initial pin record (this commit). FFI surface clean against `kx-llamacpp/src/*.rs`; Drop coverage verified for Model/Context/Sampler/Batch/LlamaBackend. |
| 2026-06-16 | `d2462f8f7ac6...` | `b9310-280-gd2462f8f7` | +671 commits. Motivation: the `gemma4uv` unified-vision projector (Gemma-4-12B omni), absent from the prior pin. **`llama.h` C API stable for all bound symbols** (only additive `llama_n_rs_seq` + a `llama_set_warmup` deprecation we don't use). **Two changes required:** (1) `mtmd_helper_bitmap_init_from_buf` gained a `placeholder` bool and now returns `mtmd_helper_bitmap_wrapper { bitmap, video_ctx }` — `kx-llamacpp/src/mtmd.rs` updated to pass `false` + take `.bitmap`; (2) upstream added a default-ON unified `llama` app (`app/`) that links `llama-server-impl` (we build `LLAMA_BUILD_SERVER=OFF`) → added `LLAMA_BUILD_APP=OFF` in `build.rs` (`tools/mtmd`/`libmtmd.a` stays built under `LLAMA_BUILD_TOOLS`). Drop coverage re-verified; no enum-discriminant or struct-layout drift in used types. |
