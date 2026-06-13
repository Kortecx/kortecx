# kortecx build orchestration.
# All commands assume the workspace root (this directory).
#
# Locally:  `just check`   — dev quick sweep (fmt + clippy + test)
#           `just ci`       — exact mirror of CI gates
#           `just deny`     — RustSec + license + bans + sources
#           `just doctor`   — preflight check (toolchain + C++ deps + submodule)

# Default recipe — show available commands.
default:
    @just --list

# ============================================================================
# Dev workflow recipes
# ============================================================================

# Quick local sweep — fmt + clippy + test. The recipe a developer runs
# before pushing. Fast subset of `ci`; skips deny/doc/ffi-link/reproducible.
check: fmt-check clippy test

# Exact mirror of the CI workflow's gates (in dependency order). Runs every
# job .github/workflows/ci.yml runs in parallel, here sequentially. Modify
# this recipe in lock-step with ci.yml.
ci: fmt-check clippy test deny doc ffi-link build-no-inference features-guard check-reproducible scale-smoke

# Verify code is formatted per rustfmt.toml. Fails on any drift.
fmt-check:
    cargo fmt --all -- --check

# Apply formatting in-place. Use locally; CI runs fmt-check instead.
fmt:
    cargo fmt --all

# Lint workspace with clippy; deny warnings (no allowed unwrap/expect in library code).
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Build all workspace crates.
build:
    cargo build --workspace --all-targets

# Run all workspace tests.
test:
    cargo test --workspace -- --nocapture

# Build documentation; deny warnings (catches broken intra-doc links).
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# ============================================================================
# Onboarding / install automation (sudo-free, opt-in)
# ============================================================================

# One-shot onboarding for the FFI-FREE runtime (Tier 0). Builds + installs the
# `kx` binary with NO C++ toolchain and NO llama.cpp submodule. NEVER runs
# sudo / brew / apt. Opt into local inference separately (`just setup-inference`).
#   just setup            # install `kx` to ~/.cargo/bin
#   INSTALL=0 just setup  # just `cargo build --release -p kx-cli` (no install)
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "kortecx setup — FFI-free runtime (Tier 0: Rust only, no C++ toolchain)"
    if ! command -v cargo >/dev/null 2>&1; then
        echo " ✗ cargo not found — install Rust from https://rustup.rs, then re-run." >&2
        exit 1
    fi
    if [ "${INSTALL:-1}" = "1" ]; then
        echo "Installing the kx binary (cargo install --path crates/kx-cli)..."
        cargo install --path crates/kx-cli
        BIN="kx"
    else
        echo "Building the kx binary (cargo build --release -p kx-cli)..."
        cargo build --release -p kx-cli
        BIN="./target/release/kx"
    fi
    echo ""
    echo " ✓ ready. Next steps:"
    echo "     ${BIN} run    --journal /tmp/kx.db --content /tmp/kx-content"
    echo "     ${BIN} replay --journal /tmp/kx.db --content /tmp/kx-content"
    echo "     ${BIN} serve  --dev-allow-local                # zero-config: auto data dir under ~/.kortecx"
    echo "     ${BIN} serve  --dev-allow-local --journal /tmp/kx.db --content /tmp/kx-content  # or pin the paths"
    echo "       (zero-config serve prints a startup banner with the resolved journal/content/catalog +"
    echo "        the gRPC/console endpoints; set KX_DATA_DIR to relocate the base data dir.)"
    echo ""
    echo "For REAL local LLM inference (opt-in; needs a C++ toolchain), run:"
    echo "     just doctor   # per-OS install hints   →   just setup-inference"

# OPT-IN inference tier (Tier 1). Inits the llama.cpp submodule and builds the FFI
# link. Requires a C++ toolchain — run `just doctor` first for per-OS hints.
# Deliberately separate from `setup` so the default flow never triggers CMake.
setup-inference:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Initializing the llama.cpp submodule (crates/kx-llamacpp-sys/llama.cpp)..."
    git submodule update --init --recursive crates/kx-llamacpp-sys/llama.cpp
    echo "Building the FFI link (runs CMake on the submodule)..."
    cargo build -p kx-llamacpp-sys --release
    cargo build -p kx-llamacpp --release
    echo " ✓ inference tier ready. Try:  just fetch-demo-model  &&  just smoke-test-with-model"

# Download the tiny demo GGUF (stories260K, ~1.2 MB) to target/models/ with
# SHA-256 verification. Idempotent: skips if already present + valid. Triggers NO
# build (does not enable the model-smoke-test feature). Feed it to the examples:
#   cargo run -p kx-llamacpp --example generate -- target/models/stories260K.gguf "Once upon a time"
fetch-demo-model:
    #!/usr/bin/env bash
    set -euo pipefail
    URL="https://huggingface.co/ggml-org/models/resolve/main/tinyllamas/stories260K.gguf"
    SHA="270cba1bd5109f42d03350f60406024560464db173c0e387d91f0426d3bd256d"
    DEST="target/models/stories260K.gguf"
    mkdir -p target/models
    if [ -f "$DEST" ] && [ "$(shasum -a 256 "$DEST" | cut -d' ' -f1)" = "$SHA" ]; then
        echo " ✓ demo model already present + verified: $DEST"
        exit 0
    fi
    echo "Downloading $URL → $DEST ..."
    rm -f "$DEST" "$DEST.partial"
    if   command -v curl >/dev/null 2>&1; then curl -fsSL "$URL" -o "$DEST.partial"
    elif command -v wget >/dev/null 2>&1; then wget -q "$URL" -O "$DEST.partial"
    else echo " ✗ neither curl nor wget found on PATH" >&2; exit 1; fi
    GOT="$(shasum -a 256 "$DEST.partial" | cut -d' ' -f1)"
    if [ "$GOT" != "$SHA" ]; then
        echo " ✗ SHA-256 mismatch: expected $SHA, got $GOT" >&2
        rm -f "$DEST.partial"; exit 1
    fi
    mv "$DEST.partial" "$DEST"
    echo " ✓ verified + saved: $DEST"

# Download a PUBLIC Qwen3 stand-in GGUF (unsloth Qwen3-0.6B-Q4_K_M, ~397 MB) to
# target/models/ with SHA-256 verification — the drop-in test vehicle for the
# Qwen3-4B agent campaign until the private finetune ships. Same `qwen3` arch +
# ChatML + native tool-calling shape as the real model. Idempotent.
#
# Then point the harness at it:
#   export KX_MODEL_NAME=qwen3-0.6b
#   export KX_MODEL_HARNESS_GGUF="$(pwd)/target/models/qwen3-0.6b-q4_k_m.gguf"
#
# For the REAL (private) Qwen3-4B q4_k_m, override the source — a SHA is REQUIRED
# (never fetch a model unverified); a gated repo also needs HF_TOKEN:
#   KX_AGENT_MODEL_URL=<hf-resolve-url> KX_AGENT_MODEL_SHA=<sha256> \
#     KX_AGENT_MODEL_DEST=target/models/qwen3-4b-q4_k_m.gguf HF_TOKEN=<tok> just fetch-agent-model
fetch-agent-model:
    #!/usr/bin/env bash
    set -euo pipefail
    URL="${KX_AGENT_MODEL_URL:-https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf}"
    SHA="${KX_AGENT_MODEL_SHA:-ac2d97712095a558e31573f62f466a3f9d93990898b0ec79d7c974c1780d524a}"
    DEST="${KX_AGENT_MODEL_DEST:-target/models/qwen3-0.6b-q4_k_m.gguf}"
    mkdir -p "$(dirname "$DEST")"
    if [ -f "$DEST" ] && [ "$(shasum -a 256 "$DEST" | cut -d' ' -f1)" = "$SHA" ]; then
        echo " ✓ agent stand-in already present + verified: $DEST"
        exit 0
    fi
    echo "Downloading $URL → $DEST ..."
    rm -f "$DEST" "$DEST.partial"
    # Token-aware fetch (no bash arrays — macOS ships bash 3.2 where an empty
    # `"${arr[@]}"` trips `set -u`).
    if command -v curl >/dev/null 2>&1; then
        if [ -n "${HF_TOKEN:-}" ]; then
            curl -fsSL -H "Authorization: Bearer ${HF_TOKEN}" "$URL" -o "$DEST.partial"
        else
            curl -fsSL "$URL" -o "$DEST.partial"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ -n "${HF_TOKEN:-}" ]; then
            wget -q --header="Authorization: Bearer ${HF_TOKEN}" "$URL" -O "$DEST.partial"
        else
            wget -q "$URL" -O "$DEST.partial"
        fi
    else
        echo " ✗ neither curl nor wget found on PATH" >&2; exit 1
    fi
    GOT="$(shasum -a 256 "$DEST.partial" | cut -d' ' -f1)"
    if [ "$GOT" != "$SHA" ]; then
        echo " ✗ SHA-256 mismatch: expected $SHA, got $GOT" >&2
        echo "   (overriding the model? pass the matching KX_AGENT_MODEL_SHA)" >&2
        rm -f "$DEST.partial"; exit 1
    fi
    mv "$DEST.partial" "$DEST"
    echo " ✓ verified + saved: $DEST"
    echo "   Point the harness at it:"
    echo "     export KX_MODEL_NAME=qwen3-0.6b"
    echo "     export KX_MODEL_HARNESS_GGUF=\"$(pwd)/$DEST\""

# The ONE-COMMAND inference serve (§2.194 guardrail): fetch the stand-in model
# if absent (idempotent, checksum-verified), then start `kx serve` with it +
# the embedded console at :50180. Model paths are DETERMINISTIC
# (target/models/ via the fetch recipes, or the KX_AGENT_MODEL_* overrides) —
# never hunt the filesystem for GGUFs. Journal/content are PINNED under
# target/serve/ here for a repeatable, inspectable layout (override:
# `just serve-inference /path/kx.db /path/blobs`). NOTE: bare `kx serve
# --dev-allow-local` is now zero-config (auto paths under ~/.kortecx); we keep
# the explicit paths here so the inference demo's state lives beside the repo.
serve-inference journal="target/serve/kx.db" content="target/serve/blobs": fetch-agent-model
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p "$(dirname "{{journal}}")" "{{content}}"
    export KX_SERVE_MODEL_GGUF="${KX_AGENT_MODEL_DEST:-$(pwd)/target/models/qwen3-0.6b-q4_k_m.gguf}"
    echo " ▶ inference serve (model: $KX_SERVE_MODEL_GGUF)"
    cargo run --release -p kx-cli --features inference,hnsw,console --bin kx -- \
        serve --journal "{{journal}}" --content "{{content}}" --dev-allow-local

# D139: build the web console's embed input — the TS SDK (the UI's file: dep)
# then the SPA production bundle into ui/dist. The exact sequence release.yml
# runs; needed BEFORE any `--features console` cargo build (build.rs embeds the
# dist at compile time and fails loudly when it's missing). Needs node >= 22.
console-dist:
    npm --prefix bindings/typescript ci
    npm --prefix bindings/typescript run build
    npm --prefix ui ci
    npm --prefix ui run build

# D139: a release-grade kx with the embedded console + the Datasets data-plane —
# exactly what the prebuilt release ships (`--features console,hnsw`; FFI-free).
console-build: console-dist
    cargo build --release -p kx-cli --features console,hnsw

# Docs-as-test gate: run the README quickstart end to end and assert the canonical
# projection digest. Builds the FFI-free `kx` binary (no C++ toolchain) and drives
# run → crash → replay → digest over temp dirs, asserting the canonical digest
# (8/8 committed) at every step. Cleans up. Fails LOUDLY on any drift — this is the
# gate that keeps the README honest. NOT part of `just ci` (a separate, fast gate).
verify-quickstart:
    #!/usr/bin/env bash
    set -euo pipefail
    CANON="7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9"
    echo "Building the FFI-free kx binary..."
    cargo build --release -p kx-cli
    KX="{{justfile_directory()}}/target/release/kx"
    WORK="$(mktemp -d)"
    trap 'rm -rf "$WORK"' EXIT
    J="$WORK/kx.db"; C="$WORK/kx-content"

    echo "[1/3] clean run → digest"
    RUN_OUT="$("$KX" run --journal "$J" --content "$C")"
    echo "    $RUN_OUT"
    if [ "${RUN_OUT%% *}" != "$CANON" ]; then
        echo " ✗ FAIL: clean-run digest ${RUN_OUT%% *} != canonical $CANON" >&2; exit 1
    fi
    case "$RUN_OUT" in *"(8/8 committed)"*) ;; *)
        echo " ✗ FAIL: expected 8/8 committed, got: $RUN_OUT" >&2; exit 1 ;; esac

    echo "[2/3] fresh crash run (aborts mid-commit; non-zero is expected)"
    rm -f "$J"; rm -rf "$C"
    set +e
    "$KX" run --journal "$J" --content "$C" --crash-at post-commit-vtc >/dev/null 2>&1
    set -e

    echo "[3/3] replay (recover) → digest, then a standalone digest fold"
    REPLAY_OUT="$("$KX" replay --journal "$J" --content "$C")"
    echo "    $REPLAY_OUT"
    DIGEST_ONLY="$("$KX" digest --journal "$J" --content "$C")"
    if [ "${REPLAY_OUT%% *}" != "$CANON" ]; then
        echo " ✗ FAIL: replay digest ${REPLAY_OUT%% *} != canonical $CANON" >&2; exit 1
    fi
    if [ "$DIGEST_ONLY" != "$CANON" ]; then
        echo " ✗ FAIL: standalone digest $DIGEST_ONLY != canonical $CANON" >&2; exit 1
    fi
    echo ""
    echo " ✓ verify-quickstart PASS — clean run, crash-then-replay, and a fresh"
    echo "   digest fold all produce the canonical digest (8/8 committed)."

# ============================================================================
# Container (Docker / OCI) recipes
# ============================================================================

# Build the FFI-free `kx` runtime image (no C++ toolchain, no llama.cpp submodule).
# Compiles from source via cargo-chef (dependency layer cached across source-only
# rebuilds). Tags `kortecx/kx:dev` by default; override with KX_IMAGE.
#   just docker-build                       # → kortecx/kx:dev
#   KX_IMAGE=ghcr.io/you/kx:v0 just docker-build
docker-build:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:dev") }} .

# Build the FFI-free image WITHOUT compiling — COPY the SHA-256-verified `kx` from
# the GitHub Release (near-zero build). Requires KX_VERSION (the Release tag).
#   KX_VERSION=v0.1.0 just docker-build-fast
docker-build-fast:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile \
      --build-arg KX_SOURCE=prebuilt \
      --build-arg KX_VERSION={{ env_var_or_default("KX_VERSION", "") }} \
      -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:dev") }} .

# Build the CPU inference image (fetches the PINNED llama.cpp inside the builder;
# needs a C++ toolchain in the BUILDER only, not at runtime). CPU-only — Metal is
# macOS-host-only and GPU/CUDA is the cloud-side seam (Dockerfile.cuda, D28).
docker-build-inference:
    DOCKER_BUILDKIT=1 docker build -f Dockerfile.inference -t {{ env_var_or_default("KX_IMAGE", "kortecx/kx:inference-cpu") }} .

# In-container docs-as-test: build the FFI-free image, then reproduce the canonical
# projection digest THROUGH the container (clean run · crash-then-replay over a
# persisted volume · read-only rootfs). The Docker analog of `verify-quickstart`,
# and what the CI `docker-smoke` job runs. Requires a working Docker daemon. NOT
# part of `just ci` (a separate, Docker-dependent gate — like verify-quickstart).
docker-smoke:
    ./scripts/docker-smoke.sh

# ============================================================================
# Policy + supply-chain recipes
# ============================================================================

# cargo-deny: RustSec advisories + license allowlist + bans + sources.
# Configuration lives in `deny.toml` (Item 4 of the repo-baseline sweep).
# CI runs the same command in the `deny` job — keep them in lock-step.
deny:
    cargo deny check

# ============================================================================
# C++ FFI recipes — the load-bearing native-build path
# ============================================================================

# Explicit C++ FFI link path: builds `kx-llamacpp-sys` (the bindgen + CMake
# layer) and `kx-llamacpp` (the safe wrapper that LINKS against the static
# archives). Mirrors the CI `ffi-link` job. Local runs use the existing
# target/ cache; CI does NOT cache target/ for this job so a broken native
# link cannot hide. Use `just clean && just ffi-link` locally to reproduce
# the CI environment.
ffi-link:
    cargo build -p kx-llamacpp-sys --release
    cargo build -p kx-llamacpp --release

# Prove the runtime installs/builds with NO native FFI in its dependency closure
# (no C++ toolchain, no llama.cpp submodule). Step 1.1 of the OSS Adoption & Trust
# track: `cargo install kx-runtime` must not require a C++ build. Asserts
# kx-llamacpp{,-sys} is absent from the runtime's normal dependency tree, then
# builds the runtime + the feature-off inference lib (the bring-your-own-backend
# path). CI runs this on a runner WITHOUT a C++ toolchain or submodule, so a
# regression that leaks the FFI back into the runtime closure fails loudly.
build-no-inference:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Asserting kx-runtime's dependency closure is FFI-free..."
    if cargo tree -p kx-runtime -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: kx-llamacpp is in kx-runtime's dependency closure (the FFI leaked into the runtime)"
        cargo tree -p kx-runtime -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in kx-runtime closure"
    echo "Asserting kx-cli's dependency closure is FFI-free (the user-facing binary installs without a C++ toolchain)..."
    if cargo tree -p kx-cli -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: kx-llamacpp is in kx-cli's dependency closure (the FFI leaked into the CLI)"
        cargo tree -p kx-cli -e normal | grep -E 'kx-llamacpp' || true
        exit 1
    fi
    echo " ✓ no kx-llamacpp in kx-cli closure"
    cargo build -p kx-runtime
    cargo build -p kx-cli
    cargo build -p kx-inference --no-default-features
    echo " ✓ build-no-inference: PASS"

# The installed-binary feature matrix stays buildable (the v0.1.0 campaign
# guard): `cargo install -p kx-cli --features hnsw` (Datasets, FFI-free) and
# `--features inference,hnsw` (the full Tier-2 install) must both CHECK, and
# the hnsw-alone closure must stay FFI-free (a C++-toolchain-less user can
# install Datasets support). The inference half only type-checks here — the
# real FFI link is `ffi-link`/`smoke-test-with-model`.
features-guard:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Asserting kx-cli --features hnsw stays FFI-free..."
    if cargo tree -p kx-cli --features hnsw -e normal | grep -qE 'kx-llamacpp'; then
        echo " ✗ FAIL: the hnsw feature dragged the FFI into kx-cli"
        exit 1
    fi
    echo " ✓ hnsw closure is FFI-free"
    cargo check -p kx-cli --features hnsw
    cargo check -p kx-cli --features inference,hnsw
    echo " ✓ features-guard: hnsw + inference,hnsw both build"

# Byte-determinism check (I1.c). Two consecutive release builds must produce
# bit-identical artifacts. Failure indicates the build is nondeterministic and
# must be fixed before the affected change can merge.
check-reproducible:
    #!/usr/bin/env bash
    set -euo pipefail
    # I1.c is path- + metadata-sensitive: without pinned crate metadata and a
    # stripped absolute workspace path, two clean builds embed path-dependent
    # bytes and differ. CI exports these in $GITHUB_ENV before calling this
    # recipe; locally we DEFAULT them (only if unset) so `just ci` / `just
    # check-reproducible` reproduce CI's result without a manual export. CI's
    # own value (using $GITHUB_WORKSPACE) is preserved when already set.
    : "${RUSTFLAGS:=--remap-path-prefix={{justfile_directory()}}= -Cmetadata=kortecx-v0}"
    export RUSTFLAGS
    # GUARDRAIL (§2.194 deterministic-homes): `cargo clean` wipes ALL of
    # target/ — including target/models/, the checksum-verified model cache the
    # fetch recipes maintain (a 2+ GB re-download per `just ci` otherwise).
    # Stash the cache across the two clean builds; restore even on failure.
    MODELS_STASH=""
    if [ -d target/models ]; then
        MODELS_STASH="$(mktemp -d)"
        mv target/models "$MODELS_STASH/models"
    fi
    restore_models() {
        if [ -n "$MODELS_STASH" ] && [ -d "$MODELS_STASH/models" ]; then
            mkdir -p target
            rm -rf target/models
            mv "$MODELS_STASH/models" target/models
            rmdir "$MODELS_STASH" 2>/dev/null || true
        fi
    }
    trap restore_models EXIT
    cargo clean
    cargo build --release --workspace
    find target/release -maxdepth 1 -type f \( -name "*.rlib" -o -name "*.a" \) -print0 \
        | sort -z | xargs -0 shasum -a 256 > /tmp/kortecx-build-1.sha256
    cargo clean
    cargo build --release --workspace
    find target/release -maxdepth 1 -type f \( -name "*.rlib" -o -name "*.a" \) -print0 \
        | sort -z | xargs -0 shasum -a 256 > /tmp/kortecx-build-2.sha256
    # Strip absolute paths in the sha256 output (they include /tmp suffixes that vary).
    sed 's|target/release/||' /tmp/kortecx-build-1.sha256 > /tmp/kortecx-build-1.norm
    sed 's|target/release/||' /tmp/kortecx-build-2.sha256 > /tmp/kortecx-build-2.norm
    diff /tmp/kortecx-build-1.norm /tmp/kortecx-build-2.norm
    echo "I1.c byte-determinism: PASS"

# Run the kx-llamacpp model-smoke-test feature: downloads a ~1.2 MB GGUF and
# runs the full safe-wrapper inference pipeline (load → tokenize → decode →
# sample → detokenize). Gated separately so the default `just ci` doesn't
# need network access.
smoke-test-with-model:
    cargo test -p kx-llamacpp --features model-smoke-test -- --nocapture

# LOCAL / manual gate (NOT a CI job): exercise the safe-wrapper inference pipeline
# with GPU offload forced ON (`KX_N_GPU_LAYERS=-1`) so an Apple-Silicon dev sees
# Metal actually used (look for `offloaded N/N layers to GPU` + `ggml_metal_*`),
# plus the q8_0 KV-cache + flash-attn determinism cases. Pairs with the AL1
# live-serve witness (`cargo test -p kx-gateway --features inference -- --ignored`).
# CPU elsewhere (CUDA is cloud-only, D28); harmless no-op without a GPU.
metal-smoke:
    KX_N_GPU_LAYERS=-1 cargo test -p kx-llamacpp --features model-smoke-test -- --nocapture

# LOCAL / manual gate (NOT a CI job): downloads a small VLM (Qwen2-VL-2B-Instruct
# GGUF + mmproj, ~1.6 GB) and runs the full IMAGE pipeline through the safe wrapper
# (load → projector → decode bitmap → tokenize → mtmd prefill → generate),
# asserting image→text non-empty + greedy determinism + fail-closed on corrupt
# bytes. Run it before a release or when touching the mtmd FFI. Deliberately kept
# OUT of CI: a 2B VLM (multi-GB download + a CPU inference run, twice) is too heavy
# for the runner, and `ffi-link` + the no-model tests already cover the Linux link
# + the dispatch/sniff/routing logic. NOT part of `just ci`.
smoke-test-multimodal:
    cargo test -p kx-llamacpp --features model-smoke-test-multimodal --release -- --nocapture

# ============================================================================
# Scale / performance gate
# ============================================================================

# Scale smoke (M2.1 + M2.2 + M2.2b + M2.x-E / D92 — the resume-availability
# invariant): fold a 25k+ Mote journal in RELEASE and assert (M2.1) the incremental
# children-index re-fold stays ~linear, (M2.2) resume-from-`FoldCheckpoint`
# reproduces the full fold exactly + its cost is bounded by live-state size under
# churn (not by journal length), (M2.2b) the SAME bound holds end-to-end over a real
# disk-backed SQLite journal + an on-disk checkpoint sidecar, and (M2.x-E / IMP-2)
# offline schema migration (`migrate_to`) stays O(entries) so resume-after-upgrade
# is not an outage, and (IMP-4 / D116) the cold-recovery projection fold stays
# O(entries) at 10^5 — the read side of the single-writer ceiling (cold resume folds
# the whole log; a super-linear fold turns a large-log resume into an outage). The
# fold cases live in `kx-projection`; the migration case in `kx-journal` (both
# llamacpp-free — no C++ FFI in this lean job). A super-linear resume is an outage,
# and resume IS the product. `--release` is REQUIRED — in a debug build the
# differential oracle re-imposes the O(n^2) full rebuild on every fold and the ratio
# assertions are skipped. Also gates (M7 / D86) the kx-catalog governance paths —
# signature register/lookup, the grant-ledger authorization fold, and the
# depth-bounded deep-chain query — all stay sub-linear / depth-bounded at 25k-50k.
# Also gates (R6) the Morphic recipe library's wide fan-out (`map_reduce` to a
# 10k-wide reduce) — deterministic compile + reproducible run + ~linear curve.
scale-smoke:
    cargo test -p kx-projection --release --test incremental_children_index -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_checkpoint -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test run_metadata_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-journal --release --test schema_evolution -- --ignored --nocapture --test-threads=1
    cargo test -p kx-capture --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-audit --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-catalog --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-fleet --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-gateway-core --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-workflow --release --test stress_fanout -- --ignored --nocapture --test-threads=1
    cargo test -p kx-dataset-hnsw --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-gateway --release --test global_tail_stress -- --ignored --nocapture --test-threads=1

# IMP-4 (D116) single-writer scale-readiness measurement spike — publish the
# single-writer journal commit ceiling + the projection-fold curve so a real number
# replaces the "qualitatively true, quantitatively unproven" placeholder (HANDOFF
# §3.9 §A). NON-GATING (not part of `just ci`): every test is `#[ignore]`, prints
# commits/s + µs/entry, and asserts only a loose catastrophic-regression floor (the
# fold-curve linearity ratio is the only gated piece, run by `scale-smoke` above).
# `--release` is REQUIRED. `KX_CEILING_HUGE=1 just bench-ceiling` adds the 10^6 tier
# (hundreds of MB RAM + on-disk WAL — local only). On-disk commits/s is platform-
# sensitive (macOS fsync is weaker than Linux) — label numbers with their environment.
bench-ceiling:
    cargo test -p kx-journal --release --test ceiling_throughput -- --ignored --nocapture --test-threads=1
    cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1

# Golden Rule 10 — the runtime profiling harness. NON-GATING (not part of `just
# ci`): captures an environment-labelled, schema-1 JSON report of the FFI-free
# runtime's client-observable costs, then re-runs the existing throughput spikes
# so a single invocation prints the full single-node picture. The `kx-profile`
# JSON lands in `target/profile/` (gitignored); COPY it into the PRIVATE
# `docs/benchmarks/YYYY-MM-DD-<topic>.json` trend record (never committed to OSS
# — SN-2). Absolute latencies are platform-sensitive (macOS fsync ≠ Linux) — the
# report's `env` block labels every number. `--release` is REQUIRED for honest
# numbers. M1 warm-up (start→SERVING) + M2 submit→Committed come from kx-profile;
# M3 fold curve + M4 commit ceiling + M5 catalog discovery come from the spikes;
# M7a react answer-settle + M7b react tool-round (PR-2d-2) need the bundled
# stdio tool bin, built first so M7b is never silently skipped.
profile iterations="8":
    cargo build --release -p kx-mcp
    cargo run --release -p kx-profile -- --iterations {{iterations}}
    cargo test -p kx-coordinator --release --test ceiling_e2e -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-catalog --release --test scale -- --ignored --nocapture --test-threads=1

# Golden Rule 10 — inference-path profiling (M6: model warm-up + TTFT +
# tokens/sec). Requires the `model-smoke-test` GGUF (auto-downloaded, ~1.2 MB,
# SHA-verified) + the llama.cpp FFI; LOCAL-only (Metal on Apple Silicon for real
# numbers — in-container CPU inference is not representative, D135). Prints the
# timing the standard smoke test now captures; copy the numbers into the private
# trend record alongside the kx-profile JSON.
profile-inference:
    cargo test -p kx-llamacpp --release --features model-smoke-test -- --nocapture

# ============================================================================
# Preflight diagnostic
# ============================================================================

# Preflight check — verify the host has every toolchain + system library the
# build needs BEFORE the first `cargo build` invocation. Useful for new-
# contributor onboarding + CI runner debugging. Each check reports green/red;
# missing optional tools are flagged as warnings, not errors.
doctor:
    #!/usr/bin/env bash
    set -uo pipefail

    OK=" ✓"
    FAIL=" ✗"
    WARN=" !"
    errors=0
    warnings=0

    check() {
        local what="$1"
        local cmd="$2"
        if eval "$cmd" >/dev/null 2>&1; then
            echo "${OK} ${what}"
            return 0
        else
            echo "${FAIL} ${what}"
            errors=$((errors + 1))
            return 1
        fi
    }

    warn_check() {
        local what="$1"
        local cmd="$2"
        if eval "$cmd" >/dev/null 2>&1; then
            echo "${OK} ${what}"
        else
            echo "${WARN} ${what}"
            warnings=$((warnings + 1))
        fi
    }

    # Print a per-OS install hint for a missing optional (Tier 1) dependency.
    # $1 = Homebrew formula (macOS, empty ⇒ Xcode CLT); $2 = apt/dnf package(s).
    os_hint() {
        case "$(uname)" in
            Darwin)
                if [ -n "$1" ]; then echo "       fix (macOS):         brew install $1"
                else                 echo "       fix (macOS):         xcode-select --install" ; fi ;;
            Linux)
                if   command -v apt-get >/dev/null 2>&1; then echo "       fix (Debian/Ubuntu): sudo apt-get install -y $2"
                elif command -v dnf     >/dev/null 2>&1; then echo "       fix (Fedora):        sudo dnf install -y $2"
                else echo "       install '$2' via your distro's package manager" ; fi ;;
            *) echo "       install it for your platform" ;;
        esac
    }

    # Like warn_check, but prints an install hint when the dep is missing.
    # $1 = label, $2 = test cmd, $3 = brew formula, $4 = apt/dnf package(s).
    warn_with_hint() {
        if eval "$2" >/dev/null 2>&1; then
            echo "${OK} $1"
        else
            echo "${WARN} $1"
            warnings=$((warnings + 1))
            os_hint "$3" "$4"
        fi
    }

    echo "kortecx preflight — TIERED"
    echo "  Tier 0 (REQUIRED): Rust only — runs kx run / replay / serve (FFI-free)."
    echo "  Tier 1 (OPTIONAL): C++ toolchain + llama.cpp submodule — local LLM inference."
    echo ""

    echo "Tier 0 — required Rust toolchain:"
    check "Rust toolchain installed and resolves to rust-toolchain.toml pin" \
        "rustup show active-toolchain"
    check "cargo on PATH" "command -v cargo"
    check "rustc on PATH" "command -v rustc"
    check "rustfmt + clippy components present" \
        "rustup component list --installed | grep -q rustfmt && rustup component list --installed | grep -q clippy"

    echo ""
    echo "Tier 1 — local LLM inference (optional; skip for the FFI-free runtime):"
    warn_with_hint "cmake on PATH" "command -v cmake" "cmake" "cmake"
    warn_with_hint "clang on PATH" "command -v clang" "llvm" "clang"

    # Platform-specific libclang + C++ toolchain checks (bindgen needs libclang;
    # the static-archive link needs a C++ compiler). Tier 1 ⇒ warnings, not errors.
    if [ "$(uname)" = "Linux" ]; then
        warn_with_hint "libclang available (bindgen)" \
            "ldconfig -p 2>/dev/null | grep -q libclang || dpkg -s libclang-dev 2>/dev/null | grep -q 'install ok installed'" \
            "llvm" "libclang-dev"
        warn_with_hint "C++ toolchain (g++)" "command -v g++" "" "build-essential"
    elif [ "$(uname)" = "Darwin" ]; then
        warn_with_hint "Xcode Command Line Tools (libclang + clang++)" "xcode-select -p" "" ""
    fi

    warn_check "llama.cpp submodule checked out (just setup-inference)" \
        "test -f crates/kx-llamacpp-sys/llama.cpp/CMakeLists.txt"

    if [ -f crates/kx-llamacpp-sys/llama.cpp/CMakeLists.txt ] && command -v git >/dev/null 2>&1; then
        pinned=$(git -C crates/kx-llamacpp-sys/llama.cpp rev-parse HEAD 2>/dev/null || echo "unknown")
        echo "   note: submodule HEAD = ${pinned}"
        echo "         see crates/kx-llamacpp-sys/PIN.md for the audit ritual on advancing the pin."
    fi

    echo ""
    echo "Optional tools:"
    warn_check "just on PATH (for just ci recipes)" "command -v just"
    warn_check "cargo-deny installed (for just deny)" "cargo deny --version"
    warn_check "git on PATH" "command -v git"

    echo ""
    if [ "${errors}" -gt 0 ]; then
        echo "${FAIL} preflight FAILED: ${errors} Tier-0 errors, ${warnings} warnings"
        echo "    Tier 0 is required to build/run the runtime — fix the errors above."
        exit 1
    elif [ "${warnings}" -gt 0 ]; then
        echo "${WARN} Tier 0 OK (the FFI-free runtime is good to go); ${warnings} Tier-1/optional warnings"
        echo "    Run \`just setup\` to install the kx binary now; the warnings only"
        echo "    matter if you want local LLM inference (\`just setup-inference\`)."
        exit 0
    else
        echo "${OK} preflight passed — Tier 0 + Tier 1 ready"
        exit 0
    fi

# ============================================================================
# Cleanup
# ============================================================================

# Wipe all build artifacts.
clean:
    cargo clean
