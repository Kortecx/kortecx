#!/usr/bin/env bash
# scripts/run-live-sweep.sh — the live-oracle sweep: LISTED vs EXECUTED vs GREEN, per file,
# per engine.
#
# WHY THIS EXISTS. A live oracle that cannot run still reports PASS. Measured: an all-green
# Ollama arm concealed 26 skips — twelve tests resolved a GGUF *path*, found none on an
# engine that has no files, printed a skip line and returned. libtest counted every one as
# passing. "The suite is green" was a reading of the harness, not of the product.
#
# So EXECUTED is its own column, and the number that matters is
#   proven = green - skip_prose
# because a test that returned early is green and proves nothing.
#
# ⚠ THIS SCRIPT IS ITSELF AN INSTRUMENT, and instruments in this repo have a habit of dying
# silently at their own zero. Every guard below is a defect that actually happened:
#   • BSD awk has no `\s` — it matches NOTHING and exits 0. A silent zero with a clean exit
#     is the worst possible combination, so `awk_selftest` proves this box's awk matches a
#     line it must match before any counting happens.
#   • `grep -c` exits 1 on a zero count, and under `set -e` that kills the assignment. Every
#     count goes through `count()`, which also refuses a non-numeric result — because a
#     missing file yields "" which arithmetic-coerces to 0 and reads as "nothing to see".
#   • `pipefail` propagates a zero-match grep through the very sentinel meant to READ zero,
#     so `set -e`/`pipefail` are deliberately NOT both on.
#   • `$?` after a pipe is the LAST command's status — every exit code is captured from a
#     non-piped command, before any `| tail`.
#   • bash 3.2 + `set -u` treats an EMPTY array expansion as an unbound variable, which
#     killed a sweep arm mid-flight. File lists are newline-delimited strings.
#   • An empty enumeration is FATAL: a sweep over zero files otherwise reports a perfect
#     green over nothing.
#   • `--nocapture` writes `test <name> ... ` WITHOUT a newline, so the result token AND
#     any skip prose land MID-LINE. Every `^`-anchored pattern therefore reads zero over a
#     log full of matches. This defeated the result parser and the skip parser in turn.
#   • A SENTINEL row (`policy_admin_e2e`, deliberately GGUF-only) must read SKIPPED on the
#     Ollama arm. If it reads executed, that arm's env never took effect and every other
#     row in the column is worthless. It caught BOTH parser bugs above before either could
#     be published — which is the entire argument for having a positive control on the
#     instrument rather than only on the subject.
#
# Usage:
#   bash scripts/run-live-sweep.sh                     # the default file set, both engines
#   bash scripts/run-live-sweep.sh --files a,b,c       # a named subset
#   bash scripts/run-live-sweep.sh --engine ollama     # one arm
#
# Env: KX_SWEEP_GGUF (llama.cpp arm), KX_SWEEP_OLLAMA_MODELS, KX_SWEEP_EMBED_MODEL.
# Output: $RUN_DIR (default ${TMPDIR:-/tmp}/kx-live-sweep) — NEVER inside target/, which
# `just ci` cargo-cleans out from under a running job.

set -uo pipefail   # NOT -e: every arm must run and the failures aggregate.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUN_DIR="${RUN_DIR:-${TMPDIR:-/tmp}/kx-live-sweep}"
rm -rf "$RUN_DIR"; mkdir -p "$RUN_DIR/ollama" "$RUN_DIR/llamacpp"
SUMMARY="$RUN_DIR/SUMMARY.tsv"

FEATURES="inference,hnsw,observability"
GGUF="${KX_SWEEP_GGUF:-$HOME/.kx-models/gemma-4-12b-it-q4_k_m.gguf}"
OLLAMA_MODELS="${KX_SWEEP_OLLAMA_MODELS:-gemma4:12b,embeddinggemma:latest}"
EMBED="${KX_SWEEP_EMBED_MODEL:-embeddinggemma:latest}"

# The deliberately single-engine file: its skip on the Ollama arm is the POSITIVE CONTROL
# that the arm's environment actually took effect.
SENTINEL="policy_admin_e2e"

# Default set. Deliberately a NAMED list rather than "every file with #[ignore]": a full
# two-engine sweep of every live oracle against a 12B model is many hours, and a runner
# that silently truncates a longer list would be exactly the kind of quiet cap this repo
# refuses. Pass --files to widen or narrow, and the report states what was covered.
FILES_DEFAULT="al1_serve
app_ide_live_serve
app_scaffold_live_serve
models_pull_live_serve
react_auto_serve
react_serve
app_live_serve
args_grammar_serve
vision_capability_live_serve
policy_admin_e2e"

FILES="$FILES_DEFAULT"
ENGINES="llamacpp ollama"

while [ $# -gt 0 ]; do
  case "$1" in
    --files)  FILES="$(printf '%s' "$2" | tr ',' '\n')"; shift 2 ;;
    --engine) ENGINES="$2"; shift 2 ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "run-live-sweep: unknown argument '$1'" >&2; exit 3 ;;
  esac
done

# ---------------------------------------------------------------------------------
# Instrument guards
# ---------------------------------------------------------------------------------

# Count matches. 0 is a RESULT, not a failure — and a non-numeric result is a BUG, not a
# zero, because "" coerces to 0 in arithmetic and reads exactly like "nothing to see".
count() {
  local n
  n=$(grep -c -e "$1" "$2" 2>/dev/null || true)
  case "$n" in
    ''|*[!0-9]*) echo "FATAL: count() produced non-numeric '$n' for pattern '$1' in '$2'" >&2; exit 3 ;;
  esac
  printf '%s\n' "$n"
}

# Prove this box's awk matches a libtest result line BEFORE trusting any zero it reports.
awk_selftest() {
  local hit
  hit=$(printf 'test some_name ... ok\n' \
        | awk '/^test[[:space:]].*[[:space:]]\.\.\.[[:space:]]ok$/{print "HIT"}')
  if [ "$hit" != "HIT" ]; then
    echo "FATAL: awk on this box does not match a libtest result line. The sweep would" >&2
    echo "       report every file as 0/0 and look like a clean run over nothing." >&2
    exit 3
  fi
}
awk_selftest

# An empty enumeration is fatal — a perfect green over nothing is the failure this
# instrument exists to prevent, and it must not be able to produce one itself.
n_files=$(printf '%s\n' "$FILES" | grep -c . || true)
case "$n_files" in ''|*[!0-9]*) n_files=0 ;; esac
if [ "$n_files" -eq 0 ]; then
  echo "FATAL: enumerated ZERO test files. Refusing to report a sweep over nothing." >&2
  exit 3
fi

printf 'engine\tfile\tlisted\tran\tgreen\tred\tskip_prose\tproven\n' > "$SUMMARY"

# ---------------------------------------------------------------------------------
# The arms
# ---------------------------------------------------------------------------------

run_arm() {
  local engine="$1" file="$2"
  local log="$RUN_DIR/$engine/$file.log"
  local list="$RUN_DIR/$engine/$file.list"

  # Rule 55: LIST on the feature set the proof uses, before running anything.
  cargo test -p kx-gateway --features "$FEATURES" --test "$file" -- --ignored --list \
    > "$list" 2>&1
  local list_rc=$?          # BEFORE any pipe.
  if [ $list_rc -ne 0 ]; then
    printf '%s\t%s\tBUILD-FAIL\t-\t-\t-\t-\t-\n' "$engine" "$file" >> "$SUMMARY"
    return
  fi
  local listed
  listed=$(count ': test$' "$list")

  if [ "$engine" = "ollama" ]; then
    env -u KX_SERVE_MODEL_GGUF -u KX_GEMMA_MODEL_DEST \
      KX_SERVE_OLLAMA=on \
      KX_SERVE_OLLAMA_MODELS="$OLLAMA_MODELS" \
      KX_SERVE_EMBED_MODEL="$EMBED" \
      cargo test -p kx-gateway --features "$FEATURES" --test "$file" \
        -- --ignored --nocapture --test-threads=1 > "$log" 2>&1
  else
    KX_SERVE_OLLAMA=off KX_SERVE_MODEL_GGUF="$GGUF" \
      cargo test -p kx-gateway --features "$FEATURES" --test "$file" \
        -- --ignored --nocapture --test-threads=1 > "$log" 2>&1
  fi
  local run_rc=$?           # BEFORE any pipe.

  # ⚠ Parse libtest's OWN summary line, not the per-test lines.
  #
  # Under `--nocapture` libtest prints `test <name> ... ` and then the test's output, so
  # the `ok`/`FAILED` token does NOT land on the same line as the name. A regex anchored
  # on `^test .* \.\.\. ok$` therefore matches NOTHING and reports a clean `ran=0` for
  # every file — a uniform zero that looks like a tidy result. Measured: the first run of
  # this script reported 0/0/0 across six arms, and only the sentinel refused to publish
  # it. The summary line is libtest's own accounting and is stable under --nocapture.
  local green red skips ran proven
  green=$(sed -n 's/^test result:.* \([0-9][0-9]*\) passed;.*/\1/p' "$log" | tail -1)
  red=$(sed -n 's/^test result:.*; \([0-9][0-9]*\) failed;.*/\1/p' "$log" | tail -1)
  case "$green" in ''|*[!0-9]*) green=0 ;; esac
  case "$red" in ''|*[!0-9]*) red=0 ;; esac
  # ⚠ NOT anchored with `^`. Under --nocapture libtest writes `test <name> ... ` WITHOUT a
  # newline and the test's own output lands on that same line, so skip prose is mid-line.
  # This is the same interleaving that defeats a `^test .* ok$` result regex, and it bit
  # this script twice before the sentinel refused to publish the column.
  skips=$(count 'skipping:\|(skip) ' "$log")
  ran=$((green + red))
  # A file that BUILT and listed tests but produced no summary line at all did not merely
  # run zero tests — it died (a panic in a shared fixture, a SIGABRT). Say so rather than
  # reporting a tidy zero.
  if [ "$listed" -gt 0 ] && [ "$ran" -eq 0 ]; then
    if ! grep -q '^test result:' "$log"; then
      printf '%s\t%s\t%s\tNO-SUMMARY\t-\t-\t-\t-\n' "$engine" "$file" "$listed" >> "$SUMMARY"
      echo "  [$engine] $file: listed=$listed but the harness produced NO test-result line (died?)"
      return
    fi
  fi
  proven=$((green - skips))
  [ "$proven" -lt 0 ] && proven=0

  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
    "$engine" "$file" "$listed" "$ran" "$green" "$red" "$skips" "$proven" >> "$SUMMARY"
  echo "  [$engine] $file: listed=$listed ran=$ran green=$green red=$red skip_prose=$skips proven=$proven (rc=$run_rc)"
}

for engine in $ENGINES; do
  echo "=== arm: $engine ==="
  if [ "$engine" = "llamacpp" ]; then
    [ -f "$GGUF" ] || { echo "FATAL: llamacpp arm needs a GGUF at $GGUF" >&2; exit 3; }
    # GPU residency is a cross-ENGINE singleton: a resident Ollama model starves the
    # in-process arm.
    ollama stop "${OLLAMA_MODELS%%,*}" >/dev/null 2>&1 || true
    sleep 2
  fi
  printf '%s\n' "$FILES" | while IFS= read -r f; do
    [ -n "$f" ] || continue
    run_arm "$engine" "$f"
  done
done

# ---------------------------------------------------------------------------------
# The sentinel: the positive control ON the instrument
# ---------------------------------------------------------------------------------
sentinel_ok=1
if printf '%s\n' "$ENGINES" | grep -q ollama && printf '%s\n' "$FILES" | grep -qx "$SENTINEL"; then
  s_skips=$(count 'skipping:\|(skip) ' "$RUN_DIR/ollama/$SENTINEL.log")
  if [ "$s_skips" -eq 0 ]; then
    sentinel_ok=0
    echo "" >&2
    echo "SENTINEL FAILED: '$SENTINEL' is deliberately GGUF-only and MUST skip on the" >&2
    echo "  Ollama arm. It did not, so that arm's environment never took effect and every" >&2
    echo "  other Ollama row above is worthless. Do not publish this column." >&2
  else
    echo ""
    echo "sentinel OK: '$SENTINEL' skipped on the Ollama arm ($s_skips skip line(s)), so"
    echo "  that arm's environment demonstrably took effect."
  fi
fi

echo ""
echo "=== SUMMARY (also at $SUMMARY) ==="
column -t -s "$(printf '\t')" "$SUMMARY" 2>/dev/null || cat "$SUMMARY"
echo ""
echo "covered: $n_files file(s) x $(printf '%s\n' "$ENGINES" | wc -w | tr -d ' ') engine arm(s)."
echo "NOT a full-tree sweep — pass --files to widen. proven = green - skip_prose."
# ⚠ The DONE file carries the real EXIT STATUS, not a flag. It briefly wrote the
# sentinel's boolean under an `exit=` label, so a clean run recorded `exit=1` — a
# supervisor reading that file would have called a good sweep a failure.
if [ "$sentinel_ok" -eq 1 ]; then
  printf 'exit=0\n' > "$RUN_DIR/DONE"
  exit 0
fi
printf 'exit=1 (sentinel failed: the Ollama arm environment did not take effect)\n' \
  > "$RUN_DIR/DONE"
exit 1
