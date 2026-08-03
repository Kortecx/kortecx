#!/usr/bin/env bash
# Per-NON-DEFAULT-FEATURE gates + rustdoc. Detached: harness background tasks get killed.
#
# `just ci` and a bare `clippy --lib` BOTH miss a non-default feature's module AND its
# tests, so each feature needs its OWN `--all-targets` clippy and its own `test` run.
# Deliberately NOT run beside `just ci`: ci's `check-reproducible` cargo-cleans target/.
#
# ⚠ THE COMMANDS BELOW ARE COPIED OUT OF `.github/workflows/ci.yml`, NOT DERIVED FROM THE
# FEATURE LIST — and the difference is not cosmetic. This script used to loop
# `for FEAT in observability inference serve-engine` with a bare
# `-p kx-gateway --features $FEAT` pair, which diverged from CI on FIVE arms: the whole
# `hosted-apps` pair, the `kx-cli --test json_contract` row, both `console` pairs
# (including the `console,hosted-apps` INTERSECTION, where the hosted-app SDK channel
# only exists), `kx-inference --features llamacpp`, and `kx-ollama`. CI also lints
# COMBINATIONS (`inference,embedded-worker,hnsw,observability`, `serve-engine,hnsw`)
# rather than single features — a bare `--features inference` compiles a configuration
# that CI never builds and misses the one it does. A local gate that stands in for CI
# has to run CI's commands; anything else is a green over a different program.
#
# Keep in lock-step with ci.yml. The step names in the comments are the anchors.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
# NEVER inside target/: `just ci`'s check-reproducible runs `cargo clean`, which deletes
# the directory this is logging to — mid-run, with the redirect's fd still open, so the
# log vanishes and the done-file write fails. That is indistinguishable from the job
# having been killed, and was read that way once already.
RUN_DIR="${RUN_DIR:-${TMPDIR:-/tmp}/kx-gates}"; DONE="$RUN_DIR/DONE-features"
mkdir -p "$RUN_DIR"; rm -f "$DONE"
exec > >(tee -a "$RUN_DIR/features.log") 2>&1
echo "══ feature gates starting $(date -u +%H:%M:%SZ) ══"
echo "   logs: $RUN_DIR"
FAIL=0
run() {  # run <label> <cmd...>
    local label="$1"; shift
    echo; echo "── $label ──"
    "$@" > "$RUN_DIR/$label.log" 2>&1
    local rc=$?
    if [ $rc -eq 0 ]; then echo "  ✓ $label"; else echo "  ✗ $label (exit $rc)"; tail -25 "$RUN_DIR/$label.log"; FAIL=1; fi
}

# KX_SERVE_OLLAMA=off is not tidiness — an ambient Ollama daemon makes
# `list_models_is_an_honest_empty_list_on_an_ffi_free_serve` fail under BOTH the
# `inference` and `serve-engine` features, because the serve then honestly reports a
# model it can reach. A/B on one commit: daemon reachable => FAILED, daemon off => ok.
# Pinning it here means the gate measures the diff instead of whatever happens to be
# listening on this machine. (Unset means `auto`, which is the OPPOSITE of CI.)
export KX_SERVE_OLLAMA=off

# --- ci.yml "hosted-apps feature gate (clippy + tests)" ------------------------
run "clippy-hosted-apps" cargo clippy -p kx-gateway --features hosted-apps --all-targets -- -D warnings
run "test-hosted-apps"   cargo test   -p kx-gateway --features hosted-apps

# --- ci.yml "observability feature gate (clippy + tests)" ----------------------
run "clippy-observability" cargo clippy -p kx-gateway --features observability --all-targets -- -D warnings
run "test-observability"   cargo test   -p kx-gateway --features observability
run "test-cli-observability-json-contract" \
    cargo test -p kx-cli --features observability --test json_contract

# --- ci.yml "Clippy + tests for the console feature (kx-gateway)" --------------
# The second pair is the console∩hosted-apps INTERSECTION: the hosted-app SDK channel
# exists only there (the gateway serves @kortecx/sdk from its console registry, and the
# supervisor pins a scaffolded project to the version being served).
run "clippy-console" cargo clippy -p kx-gateway --features console --all-targets -- -D warnings
run "test-console"   cargo test   -p kx-gateway --features console
run "clippy-console-hosted-apps" \
    cargo clippy -p kx-gateway --features console,hosted-apps --all-targets -- -D warnings
run "test-console-hosted-apps" \
    cargo test -p kx-gateway --features console,hosted-apps

# --- ci.yml "clippy under --features inference (the cfg(inference) lint gap)" ---
# A COMBINATION, not a bare feature. Needs the llama.cpp submodule (FFI).
#
# ⚠ The probe path is the SUBMODULE'S REAL LOCATION, read out of .gitmodules — not a guess.
# The first version of this line probed `vendor/llama.cpp`, a directory that does not exist
# in this repo, so the condition could never be true and the arm could never run. It failed
# LOUDLY (see the else branch) rather than silently reporting a pass, which is the only
# reason it was caught in the same session — but a guard whose input cannot exist where it
# runs is the defect regardless of how it announces itself.
FFI_SUBMODULE="$ROOT/crates/kx-llamacpp-sys/llama.cpp"
if [ -n "$(ls -A "$FFI_SUBMODULE" 2>/dev/null)" ] || [ -n "${KX_FFI_READY:-}" ]; then
    run "clippy-inference-combo" \
        cargo clippy -p kx-gateway --features inference,embedded-worker,hnsw,observability \
            --all-targets -- -D warnings
    run "clippy-kx-inference-llamacpp" \
        cargo clippy -p kx-inference --features llamacpp --all-targets -- -D warnings
    run "test-inference-combo" \
        cargo test -p kx-gateway --features inference,embedded-worker,hnsw,observability -- --nocapture
else
    echo; echo "── inference combo ──"
    echo "  ⚠ SKIPPED: the llama.cpp submodule is absent, so the FFI arms cannot build here."
    echo "    This is a REAL GAP in this run, not a pass — CI builds them. Say so when"
    echo "    reporting, and set KX_FFI_READY=1 once the submodule is checked out."
fi

# --- ci.yml "clippy + lib tests under --features serve-engine (FFI-free serve loop)" ---
run "clippy-kx-ollama" cargo clippy -p kx-ollama --all-targets -- -D warnings
run "clippy-serve-engine-hnsw" \
    cargo clippy -p kx-gateway --features serve-engine,hnsw --all-targets -- -D warnings
run "test-serve-engine-hnsw" \
    cargo test -p kx-gateway --features serve-engine,hnsw --lib -- --nocapture

# The §2 trap: a module with BOTH an outer /// on its `mod` and inner //! docs merges
# them and resolves links in the PARENT scope.
echo; echo "── rustdoc (-D warnings) ──"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features \
    > "$RUN_DIR/rustdoc.log" 2>&1
RC=$?
if [ $RC -eq 0 ]; then echo "  ✓ rustdoc"; else echo "  ✗ rustdoc (exit $RC)"; tail -30 "$RUN_DIR/rustdoc.log"; FAIL=1; fi

echo "exit=$FAIL" > "$DONE"
echo "══ feature gates finished FAIL=$FAIL $(date -u +%H:%M:%SZ) ══"
exit "$FAIL"
