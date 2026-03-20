#!/usr/bin/env bash
set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"

echo "╔══════════════════════════════════════════════════════╗"
echo "║       Kortecx — Executable Intelligence Platform     ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# ── Pre-flight checks ──────────────────────────────────────────────────────────
for cmd in docker node uv; do
  if ! command -v "$cmd" &>/dev/null; then
    echo "ERROR: $cmd not found. Please install it first."
    exit 1
  fi
done

if ! docker info &>/dev/null; then
  echo "ERROR: Docker daemon is not running. Start Docker Desktop first."
  exit 1
fi

# ── Free required ports (skip Docker-managed processes) ──────────────────────
for port in 3000 5050 8000; do
  pid=$(lsof -ti:"$port" 2>/dev/null || true)
  if [ -n "$pid" ]; then
    # Check if the process is docker/com.docker — if so, skip it
    proc_name=$(ps -p "$pid" -o comm= 2>/dev/null || true)
    if echo "$proc_name" | grep -qi docker; then
      continue
    fi
    echo "       Port $port in use (PID $pid) — killing..."
    kill "$pid" 2>/dev/null || true
    sleep 0.5
  fi
done

# ── 1. Docker services (PostgreSQL + Qdrant + MLflow) ─────────────────────────
echo "[1/6] Starting Docker services (PostgreSQL + Qdrant + MLflow)..."
docker compose -f "$ROOT/docker-compose.yml" up -d

echo "[2/6] Waiting for services to be healthy..."
retries=0
until docker compose -f "$ROOT/docker-compose.yml" exec -T db pg_isready -U kortecx -d kortecx_dev 2>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 30 ]; then
    echo "ERROR: PostgreSQL failed to become ready after 30s"
    exit 1
  fi
  sleep 1
done
echo "       PostgreSQL ready."

retries=0
until curl -sf http://localhost:6333/healthz &>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 30 ]; then
    echo "ERROR: Qdrant failed to become ready after 30s"
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

# ── 2. Frontend schema sync ───────────────────────────────────────────────────
echo "[3/6] Syncing Drizzle schema..."
cd "$ROOT/frontend"
npx drizzle-kit push 2>&1 | tail -3
cd "$ROOT"

# ── 3. Python engine ──────────────────────────────────────────────────────────
echo "[4/6] Starting Kortecx Engine (Python FastAPI)..."
cd "$ROOT/engine"
uv run uvicorn engine.main:app --host 0.0.0.0 --port 8000 &
ENGINE_PID=$!
cd "$ROOT"
echo "       Engine PID: $ENGINE_PID"

# ── 4. Frontend ────────────────────────────────────────────────────────────────
echo "[5/6] Starting Next.js frontend..."
cd "$ROOT/frontend"
exec npx next dev
