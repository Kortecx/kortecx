#!/usr/bin/env bash
# Detached dual-engine baseline recapture for `bench-v1`.
#
# Editing `suite.json` moves the suite digest, and the digest comparison fails CLOSED
# everywhere — so BOTH engine baselines must be recaptured before anything is honest
# again. Both arms run inside ONE lease window: the GPU and Ollama are singular, and
# releasing between arms invites a peer to take the lease mid-recapture and leave the
# two baselines measured under different conditions.
#
# `ollama stop` runs between the arms. A resident Ollama model starves the in-process
# llama.cpp arm of VRAM, and the symptom is a slow or failed arm that reads as model
# unreliability rather than as contention.
#
# Writes DONE at the end whatever happens, so "finished and failed" is distinguishable
# from "still running".
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RUN_DIR="${RUN_DIR:-target/recapture}"
LOG="$RUN_DIR/run.log"
DONE="$RUN_DIR/DONE"
LABEL="${KX_LEASE_LABEL:-$(basename "$PWD")}"

mkdir -p "$RUN_DIR"
rm -f "$DONE"
exec > >(tee -a "$LOG") 2>&1

echo "══ recapture starting $(date -u +%Y-%m-%dT%H:%M:%SZ) ══"
STATUS=1
RENEWER_PID=""

finish() {
    [ -n "$RENEWER_PID" ] && kill "$RENEWER_PID" 2>/dev/null || true
    ollama stop gemma3:12b >/dev/null 2>&1 || true
    bash scripts/model-lease.sh release --label "$LABEL" >/dev/null 2>&1 || true
    echo "exit=$STATUS" > "$DONE"
    echo "══ recapture finished status=$STATUS $(date -u +%Y-%m-%dT%H:%M:%SZ) ══"
}
trap finish EXIT

echo "── lease ──"
bash scripts/model-lease.sh acquire --label "$LABEL" --purpose "bench-v1 dual-engine recapture" --wait \
    || { echo "lease acquire failed"; exit 1; }

# RENEW CONTINUOUSLY, not between arms.
#
# The first version of this script renewed once, between the two engine arms. A
# single arm is 46 tasks against a 12B model and ran 3664s — well past the 2700s
# TTL — so the lease LAPSED mid-capture and the next `status` call drained it. The
# capture kept running with no lease at all, which is precisely the window where a
# peer takes the GPU and both sessions get garbage.
#
# A lease whose renewal is tied to phase boundaries expires during any phase longer
# than the TTL, and the long phase is the one that most needs protecting.
( while :; do
      sleep 600
      bash scripts/model-lease.sh renew --label "$LABEL" >/dev/null 2>&1 || true
  done ) &
RENEWER_PID=$!
echo "lease renewer running (pid $RENEWER_PID, every 600s against a 2700s TTL)"

MODEL="${KX_GEMMA_MODEL_DEST:-}"
if [ -z "$MODEL" ] || [ ! -f "$MODEL" ]; then
    for SIB in "$ROOT" "$ROOT"/../kortecx "$ROOT"/../kortecx-*; do
        CAND="$SIB/target/models/gemma-4-12b-it-q4_k_m.gguf"
        [ -f "$CAND" ] && { MODEL="$CAND"; break; }
    done
fi
[ -f "$MODEL" ] || { echo "model GGUF missing"; exit 1; }
echo "model: $MODEL"
export KX_SERVE_MODEL_GGUF="$MODEL"

echo "── preflight: the model-free gates BEFORE any paid capture ──"
cargo build -p kx-mcp -p kx-script-runner || { echo "tool bins failed to build"; exit 1; }
cargo test -p kx-gateway --test nlauthor_bench_drive || { echo "nlauthor preflight FAILED — not spending a capture on it"; exit 1; }
cargo test -p kx-gateway --features serve-engine --test workflow_bench_drive \
    || { echo "workflow preflight FAILED"; exit 1; }
echo "preflight green"

# ── ARM 1: llama.cpp (in-process) ────────────────────────────────────────────
# Ollama first stopped: a resident model would starve this arm.
echo "── arm 1/2: llama.cpp (in-process) ──"
ollama stop gemma3:12b >/dev/null 2>&1 || true
sleep 3
KX_SERVE_OLLAMA=off KX_SERVE_MEMORY=1 KX_BENCH_UPDATE_BASELINE=1 \
    cargo test -p kx-gateway --features inference,hnsw,observability \
    --test eval_bench_real -- --ignored --nocapture --test-threads=1 \
    > "$RUN_DIR/llamacpp.log" 2>&1
A1=$?
echo "llama.cpp arm exit=$A1 (tail below)"
tail -25 "$RUN_DIR/llamacpp.log"
[ "$A1" -eq 0 ] || { echo "arm 1 FAILED"; exit 1; }

# ── ARM 2: Ollama ─────────────────────────────────────────────────────────────
echo "── arm 2/2: Ollama ──"
# Ollama has no primary model to fall back to, so the dataset embedder must be named
# EXPLICITLY (`KX_SERVE_EMBED_MODEL else the primary`). Omitting it fails the reach
# ingest, and the reach tasks then drive `run_app` against an App that was never
# saved — a uniform `permission_denied("not authorized")` 553s into the capture.
#
# The knob lives in the INVOCATION, so nothing in the tree reminds you it is missing —
# which is exactly how it went missing. Assert the embedder is pulled here, before the
# spend, rather than recording the requirement in a comment nobody reads at 3am.
EMBED="${KX_SERVE_EMBED_MODEL:-embeddinggemma:latest}"
ollama list 2>/dev/null | awk '{print $1}' | grep -qx "$EMBED" || {
    echo "PRECONDITION FAILED: embedder '$EMBED' is not pulled — run \`ollama pull $EMBED\`."
    echo "Refusing arm 2: without it the reach fixtures never provision and the capture"
    echo "aborts mid-suite on a uniform \"not authorized\" that names nothing."
    exit 1
}
echo "arm 2 embedder: $EMBED (present)"
KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS="${KX_SERVE_OLLAMA_MODELS:-gemma3:12b,embeddinggemma:latest}" \
KX_SERVE_EMBED_MODEL="${KX_SERVE_EMBED_MODEL:-embeddinggemma:latest}" \
KX_SERVE_MEMORY=1 KX_BENCH_UPDATE_BASELINE=1 \
    cargo test -p kx-gateway --features inference,hnsw,observability \
    --test eval_bench_real -- --ignored --nocapture --test-threads=1 \
    > "$RUN_DIR/ollama.log" 2>&1
A2=$?
echo "Ollama arm exit=$A2 (tail below)"
tail -25 "$RUN_DIR/ollama.log"
[ "$A2" -eq 0 ] || { echo "arm 2 FAILED"; exit 1; }

echo "── both arms captured ──"
git -C "$ROOT" diff --stat -- crates/kx-eval/corpus/bench-v1/ | tail -5
STATUS=0
exit 0
