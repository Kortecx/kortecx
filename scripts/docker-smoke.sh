#!/usr/bin/env bash
#
# docker-smoke — in-container docs-as-test for the Docker channel.
#
# Builds the FFI-free image and reproduces the canonical projection digest THROUGH the
# container: a clean run, a crash-then-replay over a PERSISTED volume, and a read-only
# rootfs. Then asserts the image can actually run an agent, which needs the runtime's
# own tool binaries present — the image shipped without them, and nothing noticed,
# because the recipe that runs this file swallowed both a missing script and a failing
# one and reported "skipped" either way.
#
# FAIL-CLOSED THROUGHOUT. Every arm asserts; nothing is skipped silently; and the run
# ends with an assertion CENSUS, so "executed nothing" can never read as "passed".
#
# ⚠ NO `timeout(1)`. It is GNU coreutils and is absent on stock macOS, where a probe
# written with it exits 127 and reads as "docker is missing" — while `command -v docker`
# passes and the daemon call then HANGS forever. `bounded` below is the portable
# replacement, and the daemon probe is the first thing this script does.

set -euo pipefail

ASSERTS=0
EXPECTED_ASSERTS=12

IMAGE="${KX_IMAGE:-kortecx/kx:smoke}"
CANON="7d22d4bdfc6f68a4311f40b20f3fe7c67f4c5d2b352f3bff8722b439e94a5af9"
TOOL_BINS="kx-mcp-echo kx-mcp-calc kx-mcp-kv
kx-connector-gmail kx-connector-discord kx-connector-slack kx-connector-notion"
VOLUME="kx-smoke-$$"
WORK="$(mktemp -d)"

# Only the daemon probe may set this. Cleanup must never call docker before it does:
# with the daemon down, `docker volume rm` BLOCKS, so a trap that reached for it would
# hang the run forever — while reporting the daemon as absent. The teardown path cannot
# depend on the thing whose absence it is reporting.
DAEMON_OK=0

cleanup() {
    if [ "$DAEMON_OK" -eq 1 ]; then
        bounded 20 docker rm -f "kx-smoke-$$" >/dev/null 2>&1 || true
        bounded 20 docker volume rm -f "$VOLUME" >/dev/null 2>&1 || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

pass() { ASSERTS=$((ASSERTS + 1)); echo " ✓ $1"; }
fail() { echo " ✗ FAIL: $1" >&2; exit 1; }

# bounded <seconds> <cmd...> — run a command with a deadline, portably.
# Returns 124 on expiry (the timeout(1) convention). Kills the plain pid and waits it:
# `kill -- -$pid` fails on a `&` child, which is not a process-group leader.
bounded() {
    local secs="$1"; shift
    "$@" & local pid=$! waited=0
    while kill -0 "$pid" 2>/dev/null; do
        if [ "$waited" -ge "$secs" ]; then
            kill "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            return 124
        fi
        sleep 1
        waited=$((waited + 1))
    done
    wait "$pid"
}

echo "== docker-smoke: in-container digest reproduction + agent readiness =="

# --- 0 · the daemon, asserted FIRST and bounded -----------------------------------
command -v docker >/dev/null 2>&1 \
    || fail "docker is not on PATH — this gate cannot run"
if ! bounded 20 docker info >/dev/null 2>&1; then
    fail "the docker daemon is not reachable (or did not answer within 20s) — this gate
        cannot run. Start Docker and re-run; do NOT treat this as a pass."
fi
DAEMON_OK=1   # unlocks the docker teardown in `cleanup`
pass "docker daemon reachable"

# --- 1 · build the FFI-free image --------------------------------------------------
DOCKER_BUILDKIT=1 docker build -f Dockerfile -t "$IMAGE" . >"$WORK/build.log" 2>&1 \
    || { tail -40 "$WORK/build.log" >&2; fail "image build failed (log above)"; }
pass "FFI-free image builds ($IMAGE)"

# --- 2 · the tools the agent verb needs, IN the image ------------------------------
# The failure this catches: the image staged only the sandbox demo body, so serve-boot
# never seeded the agent recipe and `kx agent run` refused with an unrelated-looking
# permission error. Assert both resolution paths — libexec and beside `kx`.
for b in $TOOL_BINS; do
    docker run --rm --entrypoint sh "$IMAGE" -c "test -x /usr/local/libexec/kx/$b" \
        || fail "tool binary '$b' missing from /usr/local/libexec/kx in the image"
done
pass "all 7 tool binaries staged in libexec"

for b in $TOOL_BINS; do
    docker run --rm --entrypoint sh "$IMAGE" -c "command -v $b >/dev/null" \
        || fail "tool binary '$b' is not resolvable on PATH beside kx in the image"
done
pass "all 7 tool binaries resolvable beside kx"

# A NEGATIVE control for the two arms above: a name that is deliberately absent must
# be reported as absent. Without it, a `test -x` that always succeeded would pass.
if docker run --rm --entrypoint sh "$IMAGE" -c "test -x /usr/local/libexec/kx/kx-not-a-tool"; then
    fail "the libexec probe accepts a binary that does not exist — it proves nothing"
fi
pass "the libexec probe rejects an absent binary (negative control)"

# --- 3 · clean run reproduces the canonical digest ---------------------------------
# The image declares the state dirs as VOLUMEs and `kx` reads FLAGS (never the env
# vars the image sets for documentation), so every arm passes them explicitly.
J="/var/lib/kortecx/journal/kx.db"
C="/var/lib/kortecx/content"
docker volume create "$VOLUME" >/dev/null
RUN_OUT="$(docker run --rm -v "$VOLUME:/var/lib/kortecx" "$IMAGE" \
    run --journal "$J" --content "$C" 2>/dev/null || true)"
[ "${RUN_OUT%% *}" = "$CANON" ] \
    || fail "clean-run digest '${RUN_OUT%% *}' != canonical $CANON"
case "$RUN_OUT" in *"(8/8 committed)"*) ;; *)
    fail "expected 8/8 committed in-container, got: $RUN_OUT" ;; esac
pass "clean run reproduces the canonical digest in-container (8/8 committed)"

# --- 4 · crash, then replay over the PERSISTED volume ------------------------------
docker volume rm -f "$VOLUME" >/dev/null 2>&1 || true
docker volume create "$VOLUME" >/dev/null
if docker run --rm -v "$VOLUME:/var/lib/kortecx" "$IMAGE" \
    run --journal "$J" --content "$C" --crash-at post-commit-vtc >/dev/null 2>&1; then
    fail "the crash arm exited 0 — it did not crash, so the replay below proves nothing"
fi
pass "the crash arm fails as intended (so replay has something to recover)"

REPLAY_OUT="$(docker run --rm -v "$VOLUME:/var/lib/kortecx" "$IMAGE" \
    replay --journal "$J" --content "$C" 2>/dev/null || true)"
[ "${REPLAY_OUT%% *}" = "$CANON" ] \
    || fail "replay digest '${REPLAY_OUT%% *}' != canonical $CANON — exactly-once durability did not survive the container boundary"
pass "crash-then-replay reproduces the canonical digest over a persisted volume"

# --- 5 · a standalone digest fold, and a read-only rootfs --------------------------
DIGEST_ONLY="$(docker run --rm -v "$VOLUME:/var/lib/kortecx" "$IMAGE" \
    digest --journal "$J" --content "$C" 2>/dev/null || true)"
[ "$DIGEST_ONLY" = "$CANON" ] \
    || fail "standalone digest '$DIGEST_ONLY' != canonical $CANON"
pass "a standalone digest fold agrees in-container"

RO_OUT="$(docker run --rm --read-only -v "$VOLUME:/var/lib/kortecx" "$IMAGE" \
    digest --journal "$J" --content "$C" 2>/dev/null || true)"
[ "$RO_OUT" = "$CANON" ] \
    || fail "read-only rootfs digest '$RO_OUT' != canonical $CANON"
pass "reproduces the digest under --read-only (nothing writes outside the volumes)"

# --- 6 · the image runs as a non-root user ----------------------------------------
UID_OUT="$(docker run --rm --entrypoint sh "$IMAGE" -c 'id -u' 2>/dev/null || true)"
[ "$UID_OUT" = "10001" ] || fail "image runs as uid '$UID_OUT', expected the unprivileged 10001"
pass "image runs unprivileged (uid 10001)"

# --- 7 · the console is NOT in this image ------------------------------------------
# The FFI-free image is deliberately feature-light; asserting it keeps a later
# feature-set change from silently widening what this channel ships.
if docker run --rm "$IMAGE" serve --help 2>&1 | grep -q -- "--console-listen"; then
    fail "the FFI-free image advertises --console-listen — the feature set has drifted"
fi
pass "the FFI-free image ships without the console (feature set unchanged)"

# --- census ------------------------------------------------------------------------
[ "$ASSERTS" -eq "$EXPECTED_ASSERTS" ] \
    || fail "assertion census: executed $ASSERTS, expected $EXPECTED_ASSERTS — an arm was skipped"
echo "== docker-smoke PASS: $ASSERTS/$EXPECTED_ASSERTS assertions executed =="
