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

# ── 1. Docker services (PostgreSQL + Qdrant) ──────────────────────────────────
echo "[1/5] Starting Docker services (PostgreSQL + Qdrant)..."
docker compose -f "$ROOT/docker-compose.yml" up -d

echo "[2/5] Waiting for databases to be healthy..."
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

# ── 2. Frontend schema sync ───────────────────────────────────────────────────
echo "[3/5] Syncing Drizzle schema..."
cd "$ROOT/frontend"
npx drizzle-kit push 2>&1 | tail -3
cd "$ROOT"

# ── 3. Python engine ──────────────────────────────────────────────────────────
echo "[4/5] Starting Kortecx Engine (Python FastAPI)..."
cd "$ROOT/engine"
uv run uvicorn engine.main:app --host 0.0.0.0 --port 8000 &
ENGINE_PID=$!
cd "$ROOT"
echo "       Engine PID: $ENGINE_PID"

# ── 4. Frontend ────────────────────────────────────────────────────────────────
echo "[5/5] Starting Next.js frontend..."
cd "$ROOT/frontend"
exec npx next dev
