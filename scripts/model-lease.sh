#!/usr/bin/env bash
# scripts/model-lease.sh — the LOCAL MODEL back-pressure lease (Rule 44).
#
# WHY: D214 parallelized OSS development and named the GitHub merge queue as the
# serialization point — but the queue serializes the REMOTE merge. It says nothing about
# the LOCAL machine, which every session shares and which has exactly ONE of each scarce
# thing: one GPU / VRAM pool, one Ollama daemon (:11434), and — the sharp edge — ONE set
# of DEFAULT `kx serve` ports (gRPC :50151, ws :50152, console :8888). Two sessions doing
# a Rule-41 live proof at the same time therefore collide in three ways at once:
#   1. EADDRINUSE on :50151/:8888 — the second serve dies, or worse, the second session
#      CONNECTS TO THE FIRST SESSION'S SERVE and "proves" its feature against another
#      session's build. That is a SILENTLY WRONG proof, which is the one failure Rule 41
#      exists to prevent.
#   2. VRAM thrash — two 8 GB Gemma-4-12B loads on one GPU evict each other mid-run;
#      timings and pass/fail become a coin flip.
#   3. Ollama model swap — the daemon unloads model A to serve model B, so a proof that
#      asserts residency (`ollama ps`) reads another session's model.
# The e2e fixture already dodges (1) for Playwright (`freePort()` + `KX_SERVE_OLLAMA=off`),
# but a HAND-ROLLED Rule-41 serve — which is exactly how live proofs are run — uses the
# defaults and has no guard at all.
#
# WHAT: an advisory-but-checked MUTEX over "the local model". Model-BOUND work (a live
# `kx serve`, a Rule-41 proof, `--features inference` model tests) takes the lease and
# runs alone. Model-FREE work (biome/tsc/vitest/build, cargo clippy/test, the
# Ollama-off Playwright e2e) NEVER takes it and keeps running in parallel — that is the
# point: back-pressure on the scarce resource ONLY, so small changes never queue behind
# a 12B load.
#
# HOW: `mkdir` is atomic on POSIX — the winner creates the dir, everyone else fails. The
# holder writes its identity inside, and the lease EXPIRES on a TTL so a crashed session
# cannot wedge the machine forever.
#
# TTL, not PID-liveness — a correction worth recording, because the first cut of this
# script shipped the bug it is meant to prevent. It reaped a lease whose holder PID was
# gone; but an agentic session has NO long-lived anchor process — `$$` is the script's own
# shell, which exits the moment `acquire` returns, and `$PPID` is the tool's shell, which
# exits too. So the holder ALWAYS looked dead, every check reclaimed, and the mutex
# excluded NOTHING while reporting success. A guardrail that always says yes is worse than
# no guardrail: sessions trust it. Time is the only honest anchor here.
#
# Usage:
#   bash scripts/model-lease.sh acquire --label <session> [--purpose <text>] [--wait|--try] [--timeout N] [--ttl N]
#   bash scripts/model-lease.sh renew --label <session> [--ttl N]   # long proof? extend it
#   bash scripts/model-lease.sh release [--label <session>] [--force]
#   bash scripts/model-lease.sh status
#   bash scripts/model-lease.sh ports            # a COLLISION-FREE port block for this lease
#   bash scripts/model-lease.sh with --label <s> -- <cmd...>   # acquire, run, ALWAYS release
#
# Exit: 0 = held / released / free · 1 = BUSY (another session holds it) · 2 = usage error
# Portable bash (Linux + macOS, SN-7). Advisory: it guards the convention, not the kernel.
set -uo pipefail

LEASE_ROOT="${KX_LEASE_DIR:-$HOME/.kortecx/leases}"
LOCK="$LEASE_ROOT/model.lock"
META="$LOCK/holder"

die() { echo "model-lease: $*" >&2; exit 2; }
now() { date +%s; }

field() { sed -n "s/^$1=//p" "$META" 2>/dev/null | head -1; }

# EXPIRED = held longer than its TTL. A crashed session's lease drains on its own; an
# honest session that needs longer calls `renew`. Reclaim is LOUD — a silently stolen
# lease would reintroduce exactly the concurrent-serve corruption this exists to stop.
reap_if_stale() {
  [ -d "$LOCK" ] || return 0
  local since ttl age
  since="$(field since)"; ttl="$(field ttl)"
  [ -z "$since" ] && { echo "model-lease: reclaiming MALFORMED lease (no since)" >&2; rm -rf "$LOCK"; return 0; }
  age=$(( $(now) - since )); ttl="${ttl:-2700}"
  if [ "$age" -gt "$ttl" ]; then
    echo "model-lease: reclaiming EXPIRED lease — '$(field label)' held ${age}s > ttl ${ttl}s" >&2
    echo "model-lease: if that session is ALIVE it must 'renew'; if it crashed, this is the drain." >&2
    rm -rf "$LOCK"
  fi
}

show() {
  if [ -d "$LOCK" ]; then
    local since age ttl
    since="$(field since)"; ttl="$(field ttl)"
    age=$(( $(now) - ${since:-$(now)} ))
    echo "HELD  $(field label)  (${age}s held, ttl ${ttl:-2700}s, $(( ${ttl:-2700} - age ))s to expiry)"
    sed -e 's/^/  /' "$META" 2>/dev/null
  else
    echo "FREE"
  fi
}

# Deterministic per-label port block. `kx serve` defaults (50151/50152/8888) are FIXED, so
# two hand-rolled serves collide — or silently share. Hash the label into a high block so
# every session gets its own, and a proof can never reach another session's serve.
ports() {
  local label="${1:-$(basename "$PWD")}" h
  h=$(printf '%s' "$label" | cksum | cut -d' ' -f1)
  local base=$(( 51000 + (h % 400) * 10 ))
  echo "KX_LEASE_GRPC=$base"
  echo "KX_LEASE_WS=$((base + 1))"
  echo "KX_LEASE_CONSOLE=$((base + 2))"
  echo "# use: kx serve --listen 127.0.0.1:$base --ws-listen 127.0.0.1:$((base+1)) --console-listen 127.0.0.1:$((base+2))"
}

CMD="${1:-status}"; shift || true
LABEL=""; PURPOSE=""; MODE="try"; TIMEOUT=1800; TTL=2700; FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --label)   LABEL="${2:-}"; shift 2 ;;
    --purpose) PURPOSE="${2:-}"; shift 2 ;;
    --wait)    MODE="wait"; shift ;;
    --try)     MODE="try"; shift ;;
    --timeout) TIMEOUT="${2:-1800}"; shift 2 ;;
    --ttl)     TTL="${2:-2700}"; shift 2 ;;
    --force)   FORCE=1; shift ;;
    --)        shift; break ;;
    *)         break ;;
  esac
done
[ -z "$LABEL" ] && LABEL="$(basename "$PWD")"

case "$CMD" in
  status) reap_if_stale; show ;;
  ports)  ports "$LABEL" ;;

  acquire)
    mkdir -p "$LEASE_ROOT"
    local_deadline=$(( $(now) + TIMEOUT ))
    while :; do
      reap_if_stale
      if mkdir "$LOCK" 2>/dev/null; then
        { echo "label=$LABEL"; echo "since=$(now)"; echo "ttl=$TTL"; echo "pid=$$"; \
          echo "purpose=${PURPOSE:-model-bound work}"; echo "cwd=$PWD"; } > "$META"
        echo "model-lease: ACQUIRED by '$LABEL' (ttl ${TTL}s — 'renew' if the proof runs long)"
        ports "$LABEL"
        exit 0
      fi
      # Re-entrant: the same label already holds it (a nested acquire is not a deadlock).
      if [ "$(field label)" = "$LABEL" ]; then
        echo "model-lease: already held by '$LABEL' (re-entrant, no-op)"; exit 0
      fi
      if [ "$MODE" = "try" ]; then
        echo "model-lease: BUSY — held by '$(field label)' ($(field purpose))" >&2
        echo "model-lease: do MODEL-FREE work now (biome/tsc/vitest/build, cargo clippy/test," >&2
        echo "             Ollama-off Playwright) and retry, or re-run with --wait." >&2
        exit 1
      fi
      [ "$(now)" -ge "$local_deadline" ] && { echo "model-lease: TIMEOUT after ${TIMEOUT}s" >&2; exit 1; }
      sleep 10
    done
    ;;

  renew)
    [ -d "$LOCK" ] || { echo "model-lease: nothing to renew (FREE)" >&2; exit 1; }
    holder="$(field label)"
    [ "$holder" != "$LABEL" ] && { echo "model-lease: REFUSING renew — held by '$holder', not '$LABEL'" >&2; exit 1; }
    { echo "label=$LABEL"; echo "since=$(now)"; echo "ttl=$TTL"; echo "pid=$$"; \
      echo "purpose=$(field purpose)"; echo "cwd=$(field cwd)"; } > "$META"
    echo "model-lease: RENEWED by '$LABEL' (+${TTL}s)"
    ;;

  release)
    if [ ! -d "$LOCK" ]; then echo "model-lease: already free"; exit 0; fi
    holder="$(field label)"
    if [ -n "$holder" ] && [ "$holder" != "$LABEL" ] && [ "$FORCE" != 1 ]; then
      echo "model-lease: REFUSING — held by '$holder', not '$LABEL'" >&2
      echo "model-lease: if '$holder' is genuinely dead, wait for its ttl or use --force." >&2
      exit 1
    fi
    rm -rf "$LOCK"; echo "model-lease: released by '$LABEL'${FORCE:+ (forced)}"
    ;;

  with)
    [ $# -gt 0 ] || die "with: needs -- <cmd...>"
    bash "$0" acquire --label "$LABEL" --purpose "${PURPOSE:-with}" "--$MODE" --timeout "$TIMEOUT" >/dev/null || exit 1
    # ALWAYS release — a proof that dies must not wedge the machine.
    trap 'bash "$0" release --label "$LABEL" >/dev/null 2>&1' EXIT INT TERM
    "$@"; rc=$?
    bash "$0" release --label "$LABEL" >/dev/null 2>&1; trap - EXIT INT TERM
    exit $rc
    ;;

  *) die "unknown command '$CMD' (acquire|release|status|ports|with)" ;;
esac
