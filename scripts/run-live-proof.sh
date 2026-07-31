#!/usr/bin/env bash
# The detached supervisor for the Rule-41 live proof.
#
# Everything between steps happens INSIDE this script: acquire the lease, build,
# serve, wait for health, run the proof, tear down, release. A harness that drove
# these as separate foreground steps would lose the lease the moment it was killed,
# and a >15-minute step is exactly the thing that gets killed.
#
# Writes a DONE file at the end, whatever happened, so a watcher can distinguish
# "still running" from "finished and failed" — the two states that look identical
# from the outside.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RUN_DIR="${RUN_DIR:-target/proof}"
LOG="$RUN_DIR/run.log"
DONE="$RUN_DIR/DONE"
# Ports come from the LEASE, not from this script. `model-lease.sh ports` hashes the
# label into a per-session block precisely so two sessions can never collide or —
# worse — silently share a serve. Hard-coding them here would reintroduce the
# collision the lease exists to prevent, on ports the lease does not know about.
LABEL="${KX_LEASE_LABEL:-$(basename "$PWD")}"
eval "$(bash scripts/model-lease.sh ports "$LABEL" | grep '^KX_LEASE_')"
GRPC_PORT="${GRPC_PORT:-$KX_LEASE_GRPC}"
WS_PORT="${WS_PORT:-$KX_LEASE_WS}"
CONSOLE_PORT="${CONSOLE_PORT:-$KX_LEASE_CONSOLE}"

mkdir -p "$RUN_DIR"
rm -f "$DONE"
exec > >(tee -a "$LOG") 2>&1

echo "══ live proof starting $(date -u +%Y-%m-%dT%H:%M:%SZ) ══"
echo "   ports: grpc=$GRPC_PORT ws=$WS_PORT console=$CONSOLE_PORT"

STATUS=1
SERVE_PID=""

finish() {
    echo "── teardown ──"
    if [ -n "$SERVE_PID" ]; then
        kill "$SERVE_PID" 2>/dev/null || true
        for _ in $(seq 1 10); do
            kill -0 "$SERVE_PID" 2>/dev/null || break
            sleep 1
        done
        kill -9 "$SERVE_PID" 2>/dev/null || true
    fi
    bash scripts/model-lease.sh release --label "$LABEL" >/dev/null 2>&1 || true
    echo "exit=$STATUS" > "$DONE"
    echo "══ live proof finished status=$STATUS $(date -u +%Y-%m-%dT%H:%M:%SZ) ══"
}
trap finish EXIT

echo "── 1/5 lease ──"
bash scripts/model-lease.sh acquire --label "$LABEL" --purpose "nl-authoring live proof" --wait \
    || { echo "lease acquire failed"; exit 1; }
echo "lease held by $LABEL"

echo "── 2/5 build (release, inference+console) ──"
cargo build --release -p kx-cli --features inference,hnsw,console,hosted-apps --bin kx \
    || { echo "build failed"; exit 1; }
KX="$ROOT/target/release/kx"
export KX

echo "── 3/5 serve on dedicated ports ──"
for P in "$GRPC_PORT" "$WS_PORT" "$CONSOLE_PORT"; do
    if lsof -ti tcp:"$P" >/dev/null 2>&1; then
        echo "port $P already in use — refusing to kill the holder"; exit 1
    fi
done
# A fresh worktree has an EMPTY target/, so the GGUF is not here. Fall back to a
# sibling checkout's copy rather than re-downloading 7 GB — the model file is
# content-identical and read-only to us. Never SILENTLY: the path is printed, so a
# proof can be traced to the exact weights it ran on.
MODEL="${KX_GEMMA_MODEL_DEST:-$ROOT/target/models/gemma-4-12b-it-q4_k_m.gguf}"
MMPROJ="${KX_GEMMA_MMPROJ_DEST:-$ROOT/target/models/gemma-4-12b-it-mmproj-f16.gguf}"
if [ ! -f "$MODEL" ]; then
    for SIB in "$ROOT"/../kortecx "$ROOT"/../kortecx-*; do
        CAND="$SIB/target/models/gemma-4-12b-it-q4_k_m.gguf"
        if [ -f "$CAND" ]; then MODEL="$CAND"; MMPROJ="$SIB/target/models/gemma-4-12b-it-mmproj-f16.gguf"; break; fi
    done
fi
[ -f "$MODEL" ] || { echo "model GGUF missing: $MODEL (run \`just fetch-gemma-model\`)"; exit 1; }
echo "model: $MODEL"
export KX_SERVE_MODEL_GGUF="$MODEL"
[ -f "$MMPROJ" ] && export KX_SERVE_MMPROJ_GGUF="$MMPROJ"
mkdir -p "$RUN_DIR/blobs" "$RUN_DIR/catalog"
"$KX" serve --journal "$RUN_DIR/kx.db" --content "$RUN_DIR/blobs" --catalog-dir "$RUN_DIR/catalog" \
    --listen "127.0.0.1:$GRPC_PORT" --ws-listen "127.0.0.1:$WS_PORT" \
    --console-listen "127.0.0.1:$CONSOLE_PORT" --dev-allow-local > "$RUN_DIR/serve.log" 2>&1 &
SERVE_PID=$!
echo "serve pid=$SERVE_PID; waiting for health (model load can take minutes)"
HEALTHY=0
for i in $(seq 1 600); do
    if "$KX" health --endpoint "http://127.0.0.1:$GRPC_PORT" >/dev/null 2>&1; then HEALTHY=1; break; fi
    kill -0 "$SERVE_PID" 2>/dev/null || { echo "serve died during startup:"; tail -30 "$RUN_DIR/serve.log"; exit 1; }
    sleep 1
done
[ "$HEALTHY" = 1 ] || { echo "serve never became healthy"; tail -30 "$RUN_DIR/serve.log"; exit 1; }
echo "serve healthy after ~${i}s"

echo "── 4/5 the proof ──"
GRPC_PORT="$GRPC_PORT" KX="$KX" bash scripts/proof-nl-authoring.sh
STATUS=$?

echo "── 5/5 done (proof exit=$STATUS) ──"
exit "$STATUS"
