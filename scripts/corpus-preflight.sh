#!/usr/bin/env bash
# scripts/corpus-preflight.sh — mechanizes the monotonic-counter discipline (Rule 29 /
# L-012, scheme A: ONE global counter per kind + a claim-time guard). The corpus carries
# four monotonic counters that parallel lanes race because they are claimed at author
# time from the then-current main:
#   • HANDOFF section   §2.NNN   (docs/HANDOFF.md, docs/05-progress-tracker.md)
#   • decision          DNNN     (docs/design/decisions.md — major `## D` headers)
#   • learning          L-NNN    (docs/learning-ledger.md — `## L-` headers)
#   • golden rule       Rule N   (07-engineering-discipline.md — `## Rule ` headers)
# The #329/#330 collision (both claimed §2.305) is the failure this prevents.
#
# What it does: fetch the freshest main, PRINT the next-free number for each counter, and
# FAIL if this branch CLAIMS a D / Rule / L number that is already ≤ the freshest on main
# (i.e. someone else took it while you were working — rebase to the printed next-free).
# It also fails on an intra-branch DUPLICATE header. § is advisory (the 887 KB single-line
# HANDOFF makes precise per-entry diffing unreliable until the reorg shards it).
#
# Run before committing/pushing a corpus change; wire into the corpus-lane pre-push.
# Repo-aware: if the corpus files are not present (a spoke repo), it is a no-op — corpus
# edits always land as a paired PR in kortecx-core.
#
# Portable bash (Linux CI + macOS local, SN-7).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ ! -f 07-engineering-discipline.md ] || [ ! -f docs/design/decisions.md ]; then
  echo "corpus-preflight: corpus files not present — this is a spoke repo; corpus edits land in kortecx-core. Skipping."
  exit 0
fi

# Base ref: prefer a fetch-only `private` remote (spoke/clone), else `origin` (which IS
# the private corpus repo in kortecx-core). Fetch is best-effort (offline → use local).
BASE_REMOTE=""
if git remote | grep -qx private; then BASE_REMOTE=private
elif git remote | grep -qx origin; then BASE_REMOTE=origin
fi
BASE="HEAD"
if [ -n "$BASE_REMOTE" ]; then
  git fetch -q "$BASE_REMOTE" main 2>/dev/null || echo "corpus-preflight: warning — could not fetch $BASE_REMOTE/main; comparing against local ref." >&2
  git rev-parse --verify -q "$BASE_REMOTE/main" >/dev/null && BASE="$BASE_REMOTE/main"
fi
echo "corpus-preflight: base = $BASE"

# Legitimate exception: backfilling a HISTORICAL anchor (e.g. a redirect stub for a gap
# in the D-number sequence) adds a number ≤ freshest ON PURPOSE — not a forward-claim race.
# A `Corpus-Backfill: D132,D133,...` commit trailer allow-lists those exact tokens so the
# collision check skips them (mirrors lane-guard's `Cross-Lane:` override; precise, not blanket).
BACKFILL=" $(git log "$BASE"..HEAD --format='%B' 2>/dev/null | sed -n 's/^Corpus-Backfill:[[:space:]]*//p' | tr ',' ' ') "

# _max REGEX FILE...  — highest integer matching REGEX across BASE's copy of the files.
_base_max() {
  local re="$1"; shift
  local n=0 f content m
  for f in "$@"; do
    content="$(git show "$BASE:$f" 2>/dev/null || true)"
    m="$(printf '%s' "$content" | grep -oE "$re" | grep -oE '[0-9]+' | sed 's/^0*//;s/^$/0/' | sort -n | tail -1 || true)"
    [ -n "${m:-}" ] && [ "$m" -gt "$n" ] && n="$m"
  done
  echo "$n"
}
# _added REGEX FILE — integers this branch/worktree ADDS vs BASE (from `+` diff lines).
_added() {
  git diff "$BASE" -- "$2" 2>/dev/null | grep -E '^\+' | grep -v '^+++' \
    | grep -oE "$1" | grep -oE '[0-9]+' | sed 's/^0*//;s/^$/0/' | sort -n -u
}
# _dups REGEX FILE — a header number that appears more than once in the worktree file.
_dups() {
  [ -f "$2" ] || return 0
  grep -oE "$1" "$2" | grep -oE '[0-9]+' | sed 's/^0*//;s/^$/0/' | sort -n | uniq -d
}

FAIL=0
# check KIND "printf-fmt" BASE_MAX ADDED_REGEX FILE  (hard-fails on ≤-freshest claims + dups)
check() {
  local kind="$1" fmt="$2" bmax="$3" re="$4" file="$5"
  printf '  %-8s freshest=%s  next-free=%s\n' "$kind" "$bmax" "$(printf "$fmt" "$((bmax + 1))")"
  local n tok
  for n in $(_added "$re" "$file"); do
    if [ "$n" -le "$bmax" ]; then
      tok="$(printf "$fmt" "$n")"
      if printf '%s' "$BACKFILL" | grep -qF " $tok "; then
        echo "    • backfill (allow-listed): $tok is a historical anchor, not a forward claim."
        continue
      fi
      echo "    ✗ COLLISION: this branch claims $tok but main is already at $(printf "$fmt" "$bmax") — rebase to $(printf "$fmt" "$((bmax + 1))")+."
      FAIL=1
    fi
  done
  for n in $(_dups "$re" "$file"); do
    echo "    ✗ DUPLICATE: $(printf "$fmt" "$n") appears more than once in $file."
    FAIL=1
  done
}

D_MAX="$(_base_max '^## D[0-9]+' docs/design/decisions.md)"
R_MAX="$(_base_max '^## Rule [0-9]+' 07-engineering-discipline.md)"
L_MAX="$(_base_max '^## L-[0-9]+' docs/learning-ledger.md)"
S_MAX="$(_base_max '2\.[0-9]+' docs/HANDOFF.md 05-progress-tracker.md)"

echo "Next-free counters (claim these; re-run after 'git fetch' before you push):"
check "D"    'D%s'      "$D_MAX" '## D[0-9]+'      docs/design/decisions.md
check "Rule" 'Rule %s'  "$R_MAX" '## Rule [0-9]+'  07-engineering-discipline.md
check "L"    'L-%03d'   "$L_MAX" '## L-[0-9]+'     docs/learning-ledger.md
printf '  %-8s freshest=2.%s  next-free=2.%s   (advisory — verify against the freshest HANDOFF §)\n' "§" "$S_MAX" "$((S_MAX + 1))"

if [ "$FAIL" -ne 0 ]; then
  echo "corpus-preflight: FAILED — a claimed counter collides with main. Fetch + rebase to the next-free number(s) above."
  exit 1
fi
echo "corpus-preflight: OK."
