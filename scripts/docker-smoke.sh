#!/usr/bin/env bash
# docker-smoke — in-container docs-as-test for the FFI-free `kx` image.
#
# Builds the FFI-free image, then reproduces the CANONICAL projection digest
# THROUGH the container across three scenarios: a clean run, a crash-then-replay
# over a persisted volume, and a read-only rootfs. Asserting the exact digest is
# the measured "runs in Docker" claim — a green/red proof, not a badge. Tokenless
# (the engine verbs `run`/`replay`/`digest` need no server/auth).
# Shared verbatim by `just docker-smoke` and the CI `docker-smoke` job.
#
#   ./scripts/docker-smoke.sh                 # builds kortecx/kx:dev from ./Dockerfile
#   KX_IMAGE=foo KX_DOCKERFILE=Dockerfile ./scripts/docker-smoke.sh
set -euo pipefail

IMAGE="${KX_IMAGE:-kortecx/kx:dev}"
DOCKERFILE="${KX_DOCKERFILE:-Dockerfile}"
# The canonical projection digest (8/8 committed) — the same invariant asserted by
# `just verify-quickstart` (justfile) and the docs-as-test gate. Keep in lock-step.
CANON="7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9"

JRN=/var/lib/kortecx/journal/kx.db
CNT=/var/lib/kortecx/content
S="$$"
vols=()

mkvol() { local n="kx-smoke-$1-$S"; docker volume create "$n" >/dev/null; vols+=("$n"); printf '%s' "$n"; }
cleanup() { for v in "${vols[@]:-}"; do docker volume rm -f "$v" >/dev/null 2>&1 || true; done; }
trap cleanup EXIT

# `kx run` prints "<digest> (N/N committed)" on stdout; tracing logs go to stderr,
# so `2>/dev/null` leaves just the result line. ${OUT%% *} is the digest token.
assert_digest() { # $1 = label, $2 = captured stdout
  local got="${2%% *}"
  if [ "$got" != "$CANON" ]; then
    echo " ✗ $1: digest ${got} != canonical ${CANON}" >&2
    exit 1
  fi
  echo " ✓ $1: $2"
}

echo "[build] ${IMAGE}  (-f ${DOCKERFILE}, FFI-free, no submodules, no C++ toolchain)"
DOCKER_BUILDKIT=1 docker build -f "$DOCKERFILE" -t "$IMAGE" .

echo "[1/3] clean run in-container → canonical digest (8/8 committed)"
VJ="$(mkvol clean-j)"; VC="$(mkvol clean-c)"
OUT="$(docker run --rm \
  -v "$VJ:/var/lib/kortecx/journal" -v "$VC:/var/lib/kortecx/content" \
  "$IMAGE" run --journal "$JRN" --content "$CNT" 2>/dev/null)"
assert_digest "clean run" "$OUT"
case "$OUT" in *"(8/8 committed)"*) ;; *) echo " ✗ expected 8/8 committed: $OUT" >&2; exit 1 ;; esac

echo "[2/3] crash-then-replay across the container boundary → same digest"
VJ="$(mkvol crash-j)"; VC="$(mkvol crash-c)"
M=(-v "$VJ:/var/lib/kortecx/journal" -v "$VC:/var/lib/kortecx/content")
# Fresh volume; hard-abort right after a side effect commits (non-zero is expected).
docker run --rm "${M[@]}" "$IMAGE" run --journal "$JRN" --content "$CNT" --crash-at post-commit-vtc >/dev/null 2>&1 || true
# Recover from the SAME persisted journal+content (the durable boundary).
OUT="$(docker run --rm "${M[@]}" "$IMAGE" replay --journal "$JRN" --content "$CNT" 2>/dev/null)"
assert_digest "replay (recovered)" "$OUT"
DOUT="$(docker run --rm "${M[@]}" "$IMAGE" digest --journal "$JRN" --content "$CNT" 2>/dev/null)"
if [ "$DOUT" != "$CANON" ]; then echo " ✗ standalone digest ${DOUT} != ${CANON}" >&2; exit 1; fi
echo " ✓ standalone digest fold: $DOUT"

echo "[3/3] read-only rootfs (+ tmpfs /tmp), non-root uid → canonical digest"
VJ="$(mkvol ro-j)"; VC="$(mkvol ro-c)"
OUT="$(docker run --rm --read-only --tmpfs /tmp \
  -v "$VJ:/var/lib/kortecx/journal" -v "$VC:/var/lib/kortecx/content" \
  "$IMAGE" run --journal "$JRN" --content "$CNT" 2>/dev/null)"
assert_digest "read-only run" "$OUT"

echo ""
echo " ✓ docker-smoke PASS — the canonical digest reproduces IN-CONTAINER across a"
echo "   clean run, a crash-then-replay over a persisted volume, and a read-only"
echo "   rootfs. Exactly-once durability survives the container boundary."
