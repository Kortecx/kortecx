#!/usr/bin/env bash
# scripts/shared-paths.sh — the shell reader of shared-paths.toml (the OSS↔private
# boundary manifest). One source of truth so the port tool, the cmp gate, the
# leak guard, and the boundary test all agree on what is shared vs private.
#
# Subcommands:
#   paths-shared              [shared].include patterns, one per line
#   paths-private             [private_only].paths patterns, one per line
#   paths-divergent           [divergent].paths patterns, one per line
#   deny-terms                [private_only].deny_terms, one per line
#   pathspec-shared           git pathspecs for [shared].include  (e.g. :(glob)crates/**)
#   pathspec-private          git pathspecs for [private_only].paths (positive)
#   pathspec-divergent        git pathspecs for [divergent].paths (positive)
#   pathspec-classified       git pathspecs for the UNION of all three classes
#   pathspec-exclude-private  git EXCLUDE pathspecs for [private_only].paths
#   grep-regex-private        anchored ERE alternation matching private path strings
#
# Portable bash (Linux CI + macOS local, per SN-7). No gawk-only features.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/shared-paths.toml"

# _array <section> <key> — print the quoted elements of `key = [ ... ]` inside
# [section]. Elements may be one-per-line or inline; both are handled.
_array() {
  awk -v sec="[$1]" -v key="$2" '
    /^\[/        { insec = ($0 == sec) }
    insec && $0 ~ ("^[ \t]*" key "[ \t]*=") { inarr = 1 }
    inarr {
      line = $0
      while (match(line, /"[^"]*"/)) {
        print substr(line, RSTART + 1, RLENGTH - 2)
        line = substr(line, RSTART + RLENGTH)
      }
      if (index($0, "]")) inarr = 0
    }
  ' "$MANIFEST"
}

# _to_regex <pattern> — convert a manifest path glob to an anchored ERE that
# matches a repo-relative path string (for the leak guard's path scan).
#   foo/**     -> ^foo/
#   00-*.md    -> ^00-[^/]*\.md$
#   CLAUDE.md  -> ^CLAUDE\.md$
_to_regex() {
  local p="$1"
  case "$p" in
    */\*\*)
      printf '^%s\n' "${p%\*\*}"
      ;;
    *\**)
      local e="${p//./\\.}"
      e="${e//\*/[^/]*}"
      printf '^%s$\n' "$e"
      ;;
    *)
      printf '^%s$\n' "${p//./\\.}"
      ;;
  esac
}

case "${1:-}" in
  paths-shared)    _array shared include ;;
  paths-private)   _array private_only paths ;;
  paths-divergent) _array divergent paths ;;
  deny-terms)      _array private_only deny_terms ;;
  pathspec-shared)
    _array shared include | while IFS= read -r p; do printf ':(glob)%s\n' "$p"; done ;;
  pathspec-private)
    _array private_only paths | while IFS= read -r p; do printf ':(glob)%s\n' "$p"; done ;;
  pathspec-divergent)
    _array divergent paths | while IFS= read -r p; do printf ':(glob)%s\n' "$p"; done ;;
  pathspec-classified)
    # The UNION of all three classes — every tracked file must match one of these,
    # else it is UNCLASSIFIED and silently escapes the mirror (L-029 completeness).
    { _array shared include; _array private_only paths; _array divergent paths; } \
      | while IFS= read -r p; do printf ':(glob)%s\n' "$p"; done ;;
  pathspec-exclude-private)
    _array private_only paths | while IFS= read -r p; do printf ':(exclude,glob)%s\n' "$p"; done ;;
  grep-regex-private)
    _array private_only paths | while IFS= read -r p; do _to_regex "$p"; done | paste -sd'|' - ;;
  *)
    echo "usage: $0 {paths-shared|paths-private|paths-divergent|deny-terms|pathspec-shared|pathspec-private|pathspec-divergent|pathspec-classified|pathspec-exclude-private|grep-regex-private}" >&2
    exit 2 ;;
esac
