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
ci: fmt-check clippy test deny doc ffi-link build-no-inference check-reproducible scale-smoke

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
scale-smoke:
    cargo test -p kx-projection --release --test incremental_children_index -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_checkpoint -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test run_metadata_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-projection --release --test fold_curve_scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-journal --release --test schema_evolution -- --ignored --nocapture --test-threads=1
    cargo test -p kx-capture --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-catalog --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-fleet --release --test scale -- --ignored --nocapture --test-threads=1
    cargo test -p kx-gateway-core --release --test scale -- --ignored --nocapture --test-threads=1

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
    check "crates/kx-llamacpp-sys/llama.cpp/ checked out (CMakeLists.txt present)" \
        "test -f crates/kx-llamacpp-sys/llama.cpp/CMakeLists.txt"
    check "submodule HEAD readable" \
        "git -C crates/kx-llamacpp-sys/llama.cpp rev-parse HEAD"

    if [ -f crates/kx-llamacpp-sys/PIN.md ] && command -v git >/dev/null 2>&1; then
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
