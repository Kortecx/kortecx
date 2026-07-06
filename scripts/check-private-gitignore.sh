#!/usr/bin/env bash
# scripts/check-private-gitignore.sh — defense-in-depth: every [private_only] root
# in shared-paths.toml should ALSO be backstopped by the OSS repo's .gitignore, so a
# private file can never be `git add`ed into public even if the leak-check is skipped.
# This is the guard that would have caught the D14 gap (a stale OSS .gitignore that
# stopped at /06-*.md and omitted the ledgers).
#
# .gitignore is a [divergent] path (each repo carries its own), so this check is
# REPO-AWARE:
#   • Private corpus repo (kortecx-core) — the corpus is intentionally TRACKED here,
#     so the manifest roots are NOT expected to be gitignored. Always advisory/skip.
#   • OSS public repo (Kortecx/kortecx) — every private root MUST be gitignored.
#     Reported as gaps; `--strict` turns gaps into a non-zero exit.
#
# Default is ADVISORY (exit 0, print gaps) so it can be wired into `just leak-check`
# without breaking CI before the one-time OSS-side .gitignore backfill; flip callers
# to `--strict` once the OSS .gitignore covers every root.
#
# Portable bash (Linux CI + macOS local, SN-7).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HERE="$(dirname "${BASH_SOURCE[0]}")"
STRICT=0
[ "${1:-}" = "--strict" ] && STRICT=1

# Repo identity: the sentinel 00-vision-and-principles.md is tracked ONLY in the
# private corpus repo (same trick as crates/kx-cli/tests/shared_boundary.rs).
sentinel="$(git -C "$ROOT" ls-files -- 00-vision-and-principles.md 2>/dev/null || true)"
if [ -n "$sentinel" ]; then
  echo "check-private-gitignore: private corpus repo (corpus is tracked) — advisory only, nothing to enforce."
  exit 0
fi

gaps=0
while IFS= read -r p; do
  [ -z "$p" ] && continue
  # A representative path for the glob: strip a trailing /** ; replace a leading
  # 00-*.md style wildcard with a concrete sample so git check-ignore can evaluate it.
  sample="$p"
  case "$p" in
    */\*\*) sample="${p%\*\*}sentinel" ;;   # foo/**  -> foo/sentinel
    *\**)   sample="${p//\*/x}" ;;          # 00-*.md -> 00-x.md
  esac
  if ! git -C "$ROOT" check-ignore -q "$sample" 2>/dev/null; then
    echo "  GAP: [private_only] root '$p' is NOT covered by .gitignore (sample: $sample)"
    gaps=$((gaps + 1))
  fi
done < <("$HERE/shared-paths.sh" paths-private)

if [ "$gaps" -eq 0 ]; then
  echo "check-private-gitignore: OK — .gitignore backstops every [private_only] root."
  exit 0
fi

echo "check-private-gitignore: $gaps private root(s) not backstopped by .gitignore."
if [ "$STRICT" -eq 1 ]; then
  echo "  (--strict) failing. Add the missing roots to the OSS .gitignore."
  exit 1
fi
echo "  (advisory) not failing. Run with --strict once the OSS .gitignore is complete."
exit 0
