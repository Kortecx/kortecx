#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"

# ── Progress bar ─────────────────────────────────────────────────────────────
TOTAL_STEPS=9
CURRENT_STEP=0
BAR_WIDTH=40

progress_bar() {
  local desc="$1"
  CURRENT_STEP=$((CURRENT_STEP + 1))
  local filled=$((CURRENT_STEP * BAR_WIDTH / TOTAL_STEPS))
  local empty=$((BAR_WIDTH - filled))
  local fill_str="" empty_str=""
  local i=0
  while [ $i -lt $filled ]; do fill_str+="█"; i=$((i + 1)); done
  i=0
  while [ $i -lt $empty ]; do empty_str+="░"; i=$((i + 1)); done
  printf "\033[32m[%s\033[90m%s\033[0m] %d/%d — %s\n" \
    "$fill_str" "$empty_str" "$CURRENT_STEP" "$TOTAL_STEPS" "$desc"
}

echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║       Kortecx — Executable Intelligence Platform     ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# ── 1. Pre-flight checks ────────────────────────────────────────────────────
progress_bar "Checking prerequisites (docker, node, uv)..."
for cmd in docker node uv; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "  ERROR: $cmd not found. Please install it first."
    exit 1
  fi
done

if ! docker info &>/dev/null; then
  echo "  ERROR: Docker daemon is not running. Start Docker Desktop first."
  exit 1
fi

# ── 2. Environment bootstrap ────────────────────────────────────────────────
progress_bar "Setting up environment files..."
if [ ! -f "$ROOT/.env" ]; then
  cp "$ROOT/.env.example" "$ROOT/.env"
  echo "       Created .env from .env.example"
fi
if [ ! -f "$ROOT/engine/.env" ]; then
  cp "$ROOT/engine/.env.example" "$ROOT/engine/.env"
  echo "       Created engine/.env from .env.example"
fi

# shellcheck disable=SC2046
export $(grep -v '^#' "$ROOT/.env" | xargs)

# ── 3. Free required ports ──────────────────────────────────────────────────
progress_bar "Freeing required ports (3000, 5050, 8000)..."
for port in 3000 5050 8000; do
  pid=$(lsof -ti:"$port" 2>/dev/null || true)
  if [ -n "$pid" ]; then
    proc_name=$(ps -p "$pid" -o comm= 2>/dev/null || true)
    if echo "$proc_name" | grep -qi docker; then
      continue
    fi
    echo "       Port $port in use (PID $pid) — killing..."
    kill "$pid" 2>/dev/null || true
    sleep 0.5
  fi
done

# ── 4. Docker services ──────────────────────────────────────────────────────
progress_bar "Starting Docker services (PostgreSQL + Qdrant + MLflow)..."
docker compose -f "$ROOT/docker-compose.yml" up -d

# ── 5. Wait for services ────────────────────────────────────────────────────
progress_bar "Waiting for services to be healthy..."
retries=0
until docker compose -f "$ROOT/docker-compose.yml" exec -T db pg_isready -U kortecx -d kortecx_dev 2>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 30 ]; then
    echo "  ERROR: PostgreSQL failed to become ready after 30s"
    exit 1
  fi
  sleep 1
done
echo "       PostgreSQL ready."

retries=0
until curl -sf http://localhost:6333/healthz &>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 30 ]; then
    echo "  ERROR: Qdrant failed to become ready after 30s"
    exit 1
  fi
  sleep 1
done
echo "       Qdrant ready."

retries=0
until curl -sf http://localhost:5050/ &>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 20 ]; then
    echo "       MLflow not ready (non-blocking) — continuing..."
    break
  fi
  sleep 1
done
[ "$retries" -lt 20 ] && echo "       MLflow ready."

# ── 6. Install dependencies ─────────────────────────────────────────────────
progress_bar "Installing dependencies..."
npm install --prefix "$ROOT/frontend" --silent
echo "       Frontend dependencies ready."

uv sync --project "$ROOT/engine" --quiet
echo "       Engine dependencies ready."

# ── 7. Frontend DB migrations ───────────────────────────────────────────────
progress_bar "Running frontend database migrations..."
cd "$ROOT/frontend"
npx tsx scripts/migrate.ts
cd "$ROOT"
echo "       Frontend schema migrations applied."

# ── 8. Engine DB migrations ─────────────────────────────────────────────────
progress_bar "Applying engine database migrations..."
docker compose -f "$ROOT/docker-compose.yml" exec -T db \
  psql -U kortecx -d kortecx_dev -q -f /dev/stdin < "$ROOT/engine/migrations/quorum.sql" 2>&1 | tail -3
echo "       Engine schema migrations applied."

# ── 9. Start engine + frontend ──────────────────────────────────────────────
progress_bar "Starting Kortecx Engine & Frontend..."
cd "$ROOT/engine"
uv run uvicorn engine.main:app --host 0.0.0.0 --port 8000 &
ENGINE_PID=$!
cd "$ROOT"
echo "       Engine PID: $ENGINE_PID"

echo ""
printf "\033[32m[%s] %d/%d — All systems go!\033[0m\n" \
  "$(i=0; s=''; while [ $i -lt $BAR_WIDTH ]; do s+='█'; i=$((i + 1)); done; echo "$s")" \
  "$TOTAL_STEPS" "$TOTAL_STEPS"
echo ""

cd "$ROOT/frontend"
exec npx next dev
