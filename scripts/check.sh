#!/usr/bin/env bash
#
# Kortecx Pre-Push Quality Gate
# Runs all linting, type checking, and tests across Python and TypeScript.
# Exit code 0 = all clear, non-zero = blocked.
#
# Usage:
#   ./scripts/check.sh          # run all checks
#   ./scripts/check.sh --lint   # lint only
#   ./scripts/check.sh --test   # tests only
#   ./scripts/check.sh --quick  # lint + typecheck (no tests)
#
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FRONTEND="$ROOT/frontend"
ENGINE="$ROOT/engine"


RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color
BOLD='\033[1m'

PASS=0
FAIL=0
SKIP=0
RESULTS=()

# ── Helpers ──────────────────────────────────────────

run_check() {
  local name="$1"
  shift
  printf "${CYAN}  ▸ %-40s${NC}" "$name"
  if output=$("$@" 2>&1); then
    printf "${GREEN}PASS${NC}\n"
    PASS=$((PASS + 1))
    RESULTS+=("PASS|$name")
  else
    printf "${RED}FAIL${NC}\n"
    echo "$output" | head -20 | sed 's/^/    /'
    FAIL=$((FAIL + 1))
    RESULTS+=("FAIL|$name")
  fi
}

skip_check() {
  local name="$1"
  local reason="$2"
  printf "${CYAN}  ▸ %-40s${YELLOW}SKIP${NC} %s\n" "$name" "$reason"
  SKIP=$((SKIP + 1))
  RESULTS+=("SKIP|$name")
}

section() {
  echo ""
  printf "${BOLD}${BLUE}── %s ──${NC}\n" "$1"
}

# ── Parse args ───────────────────────────────────────

MODE="all"
if [[ "${1:-}" == "--lint" ]]; then MODE="lint"; fi
if [[ "${1:-}" == "--test" ]]; then MODE="test"; fi
if [[ "${1:-}" == "--quick" ]]; then MODE="quick"; fi

echo ""
printf "${BOLD}Kortecx Quality Gate${NC}  (mode: ${YELLOW}$MODE${NC})\n"
echo "═══════════════════════════════════════════════════"

# ═══════════════════════════════════════════════════════
# LINT & TYPE CHECKS
# ═══════════════════════════════════════════════════════

if [[ "$MODE" != "test" ]]; then

  section "TypeScript (frontend/)"
  cd "$FRONTEND"
  run_check "tsc --noEmit" npx tsc --noEmit
  run_check "eslint (errors only)" npx eslint . --ext .ts,.tsx --quiet

  section "Python (engine/)"
  cd "$ENGINE"
  if command -v uv &>/dev/null; then
    run_check "ruff check" uv run --project "$ENGINE" ruff check src/
    run_check "ruff format --check" uv run --project "$ENGINE" ruff format --check src/
  else
    skip_check "ruff check" "uv not installed"
    skip_check "ruff format" "uv not installed"
  fi

fi

# ═══════════════════════════════════════════════════════
# TESTS
# ═══════════════════════════════════════════════════════

if [[ "$MODE" != "lint" && "$MODE" != "quick" ]]; then

  section "Python Tests (engine/)"
  cd "$ENGINE"
  if command -v uv &>/dev/null; then
    run_check "pytest (all tests)" uv run --project "$ENGINE" pytest tests/ -q --tb=short --no-header
  else
    skip_check "pytest" "uv not installed"
  fi

  section "Frontend Tests (frontend/)"
  cd "$FRONTEND"
  if [[ -f node_modules/.bin/vitest ]]; then
    run_check "vitest run" npx vitest run --reporter=verbose
  else
    skip_check "vitest" "node_modules not installed"
  fi

fi

# ═══════════════════════════════════════════════════════
# BUILD CHECK (full mode only)
# ═══════════════════════════════════════════════════════

if [[ "$MODE" == "all" ]]; then

  section "Build Verification"
  cd "$FRONTEND"
  run_check "next build" npx next build

fi

# ═══════════════════════════════════════════════════════
# SUMMARY
# ═══════════════════════════════════════════════════════

echo ""
echo "═══════════════════════════════════════════════════"
printf "${BOLD}Results${NC}\n"
echo "───────────────────────────────────────────────────"

for r in "${RESULTS[@]}"; do
  status="${r%%|*}"
  name="${r#*|}"
  case "$status" in
    PASS) printf "  ${GREEN}✓${NC} %s\n" "$name" ;;
    FAIL) printf "  ${RED}✗${NC} %s\n" "$name" ;;
    SKIP) printf "  ${YELLOW}○${NC} %s\n" "$name" ;;
  esac
done

echo "───────────────────────────────────────────────────"
TOTAL=$((PASS + FAIL + SKIP))
printf "  ${GREEN}Pass: $PASS${NC}  ${RED}Fail: $FAIL${NC}  ${YELLOW}Skip: $SKIP${NC}  Total: $TOTAL\n"
echo "═══════════════════════════════════════════════════"

if [[ $FAIL -gt 0 ]]; then
  echo ""
  printf "${RED}${BOLD}✗ Quality gate FAILED — fix errors before pushing.${NC}\n"
  echo ""
  exit 1
else
  echo ""
  printf "${GREEN}${BOLD}✓ All checks passed — safe to push.${NC}\n"
  echo ""
  exit 0
fi
