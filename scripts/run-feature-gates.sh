#!/usr/bin/env bash
# Per-NON-DEFAULT-FEATURE gates + rustdoc. Detached: harness background tasks get killed.
#
# `just ci` and a bare `clippy --lib` BOTH miss a non-default feature's module AND its
# tests, so each feature needs its OWN `--all-targets` clippy and its own `test` run.
# Deliberately NOT run beside `just ci`: ci's `check-reproducible` cargo-cleans target/.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
RUN_DIR="${RUN_DIR:-target/gates}"; DONE="$RUN_DIR/DONE-features"
mkdir -p "$RUN_DIR"; rm -f "$DONE"
exec > >(tee -a "$RUN_DIR/features.log") 2>&1
echo "══ feature gates starting $(date -u +%H:%M:%SZ) ══"
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
# listening on this machine.
export KX_SERVE_OLLAMA=off

for FEAT in observability inference serve-engine; do
    run "clippy-$FEAT" cargo clippy -p kx-gateway --features "$FEAT" --all-targets -- -D warnings
    run "test-$FEAT"   cargo test   -p kx-gateway --features "$FEAT"
done

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
