# kortecx build orchestration.
# All commands assume the workspace root (this directory).
# CI runs `just ci`; humans typically run `just test` or `just check-reproducible`.

# Default recipe — show available commands.
default:
    @just --list

# Format check + lint + build + test + doc + reproducibility check.
# The composite gate for P1.1's exit gate per 01-build-sequence.md §1.1.
ci: fmt-check clippy build test doc check-reproducible

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

# Wipe all build artifacts.
clean:
    cargo clean
