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
ci: fmt-check clippy test deny doc ffi-link check-reproducible

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

# Byte-determinism check (I1.c). Two consecutive release builds must produce
# bit-identical artifacts. Failure indicates the build is nondeterministic and
# must be fixed before the affected change can merge.
check-reproducible:
    #!/usr/bin/env bash
    set -euo pipefail
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

    echo "kortecx preflight — toolchain + C++ deps + submodule"
    echo ""

    echo "Required Rust toolchain:"
    check "Rust toolchain installed and resolves to rust-toolchain.toml pin" \
        "rustup show active-toolchain"
    check "cargo on PATH" "command -v cargo"
    check "rustc on PATH" "command -v rustc"
    check "rustfmt + clippy components present" \
        "rustup component list --installed | grep -q rustfmt && rustup component list --installed | grep -q clippy"

    echo ""
    echo "Native build prerequisites (kx-llamacpp-sys CMake build):"
    check "cmake on PATH" "command -v cmake"
    check "clang on PATH" "command -v clang"

    # Platform-specific libclang check (bindgen requires libclang for header parsing).
    if [ "$(uname)" = "Linux" ]; then
        warn_check "libclang available (Linux — apt: libclang-dev)" \
            "ldconfig -p 2>/dev/null | grep -q libclang || dpkg -s libclang-dev 2>/dev/null | grep -q 'install ok installed'"
    elif [ "$(uname)" = "Darwin" ]; then
        warn_check "libclang available (macOS — Xcode CLT)" "xcode-select -p"
    fi

    # C++ stdlib link target (used by the static-archive link path).
    if [ "$(uname)" = "Linux" ]; then
        check "C++ toolchain present (Linux — build-essential)" "command -v g++"
    elif [ "$(uname)" = "Darwin" ]; then
        check "C++ toolchain present (macOS — Xcode CLT)" "command -v clang++"
    fi

    echo ""
    echo "C++ FFI submodule (llama.cpp):"
    check "kx-llamacpp-sys/llama.cpp/ checked out (CMakeLists.txt present)" \
        "test -f kx-llamacpp-sys/llama.cpp/CMakeLists.txt"
    check "submodule HEAD readable" \
        "git -C kx-llamacpp-sys/llama.cpp rev-parse HEAD"

    if [ -f kx-llamacpp-sys/PIN.md ] && command -v git >/dev/null 2>&1; then
        pinned=$(git -C kx-llamacpp-sys/llama.cpp rev-parse HEAD 2>/dev/null || echo "unknown")
        echo "   note: submodule HEAD = ${pinned}"
        echo "         see kx-llamacpp-sys/PIN.md for the audit ritual on advancing the pin."
    fi

    echo ""
    echo "Optional tools:"
    warn_check "just on PATH (for just ci recipes)" "command -v just"
    warn_check "cargo-deny installed (for just deny)" "cargo deny --version"
    warn_check "git on PATH" "command -v git"

    echo ""
    if [ "${errors}" -gt 0 ]; then
        echo "${FAIL} preflight FAILED: ${errors} errors, ${warnings} warnings"
        exit 1
    elif [ "${warnings}" -gt 0 ]; then
        echo "${WARN} preflight passed with ${warnings} warnings (optional tools missing)"
        exit 0
    else
        echo "${OK} preflight passed"
        exit 0
    fi

# ============================================================================
# Cleanup
# ============================================================================

# Wipe all build artifacts.
clean:
    cargo clean
