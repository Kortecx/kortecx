#!/usr/bin/env bash
# Print the CHANGELOG section for a release tag, or nothing when there is none.
#
# The release workflow prefers these curated notes over `gh release create
# --generate-notes`: the notes are derived from the feature ledger and say what the
# release IS, where a generated list of PR titles says which branches merged.
#
# `v0.2.0-rc.1` matches the `## [0.2.0-rc.1]` heading — the tag's leading `v` is the
# only transformation. Absence is not an error: the caller falls back to generated
# notes, so an untagged-from-CHANGELOG release still publishes.
set -euo pipefail

tag="${1:?usage: extract-notes.sh <tag> [changelog]}"
changelog="${2:-CHANGELOG.md}"
version="${tag#v}"

awk -v want="## [${version}]" '
    index($0, want) == 1 { inside = 1; next }
    inside && /^## \[/    { exit }
    inside                { print }
' "$changelog"
