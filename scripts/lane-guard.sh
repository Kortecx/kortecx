#!/usr/bin/env bash
# scripts/lane-guard.sh — the no-cross-merge gate (Golden Rule 25: lane isolation).
# OSS and cloud are developed as two ISOLATED lanes; a change must not cross the
# OSS↔cloud boundary unless explicitly requested. This gate enforces it in CI on
# BOTH repos (it is [shared], invoked from each repo's divergent workflow).
#
# Lane is derived from a `Lane:` commit trailer, else the branch prefix:
#   oss     feat/*, fix/*, sync-oss-pr*  — OSS shared surface; may touch [shared] +
#                                          [divergent], NEVER [private_only]/cloud.
#   seam    feat/seam-*                  — the cloud→OSS "push the seam" exception;
#                                          same path rules as oss (an OSS change that
#                                          enables a cloud feature; impl stays private).
#   cloud   cloud/*                      — cloud repo; may touch ONLY kx-cloud/**.
#   corpus  corpus/*, mirror/*           — private corpus; no OSS-side path constraint.
#
# Explicit override ("unless specified or requested"): a `Cross-Lane: <reason>`
# trailer on any PR commit downgrades the gate to ADVISORY (warn, do not fail) and
# is surfaced in the log for the reviewer.
#
# Usage: scripts/lane-guard.sh [base-ref]   (default origin/main)
set -euo pipefail

BASE="${1:-origin/main}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# PR branch: prefer the CI-provided head ref; fall back to the local branch.
branch="${GITHUB_HEAD_REF:-$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo HEAD)}"
msgs="$(git log "${BASE}..HEAD" --format='%B' 2>/dev/null || true)"

lane="$(printf '%s\n' "$msgs" | sed -n 's/^Lane:[[:space:]]*//p' | head -1)"
if [ -z "$lane" ]; then
  case "$branch" in
    feat/seam-*)         lane=seam ;;
    cloud/*)             lane=cloud ;;
    corpus/*|mirror/*)   lane=corpus ;;
    *)                   lane=oss ;;
  esac
fi
override="$(printf '%s\n' "$msgs" | grep -c '^Cross-Lane:' || true)"

changed="$(git diff --name-only "${BASE}...HEAD")"
priv_re="$(bash "$ROOT/scripts/shared-paths.sh" grep-regex-private)"

violation=""
case "$lane" in
  oss|seam)
    hits="$(printf '%s\n' "$changed" | grep -E "$priv_re" || true)"
    [ -n "$hits" ] && violation="OSS '${lane}' lane touches private-only / cloud path(s):
$(printf '%s\n' "$hits" | sed 's/^/    /')"
    ;;
  cloud)
    hits="$(printf '%s\n' "$changed" | grep -vE '^kx-cloud/' | grep -v '^$' || true)"
    [ -n "$hits" ] && violation="cloud lane touches non-kx-cloud (shared OSS) path(s) — those edits must go through the OSS lane:
$(printf '%s\n' "$hits" | sed 's/^/    /')"
    ;;
  corpus)
    : # private corpus changes carry no OSS-side path constraint
    ;;
  *)
    echo "lane-guard: unknown lane '$lane'"; exit 2 ;;
esac

echo "lane-guard: branch='${branch}' lane='${lane}' base='${BASE}'"
if [ -n "$violation" ]; then
  echo "❌ LANE-GUARD: ${violation}"
  if [ "${override:-0}" -gt 0 ]; then
    echo "⚠️  allowed by an explicit 'Cross-Lane:' override — recorded as advisory, NOT a hard pass."
    exit 0
  fi
  echo ""
  echo "OSS and cloud functionality must not be cross-merged (GR25). Either:"
  echo "  - move the offending change to its own lane/repo, or"
  echo "  - if this crossing is intentional, add a 'Cross-Lane: <reason>' trailer."
  exit 1
fi
echo "✅ lane-guard clean — change stays within the '${lane}' lane."
