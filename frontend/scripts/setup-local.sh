#!/usr/bin/env bash
set -e

FRONTEND_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$FRONTEND_DIR/.." && pwd)"

echo "Kortecx Frontend — Local Dev Startup"
echo "─────────────────────────────────────"

# Check docker
if ! command -v docker &>/dev/null; then
  echo "Docker not found. Install from https://docs.docker.com/get-docker/"
  exit 1
fi

if ! docker info &>/dev/null; then
  echo "Docker daemon is not running. Please start Docker Desktop first."
  exit 1
fi

# Start postgres container (compose file is at project root)
echo "[1/3] Starting PostgreSQL container..."
docker compose -f "$ROOT/docker-compose.yml" up db -d

# Wait for healthy
echo "[2/3] Waiting for database to be ready..."
retries=0
until docker compose -f "$ROOT/docker-compose.yml" exec -T db pg_isready -U kortecx -d kortecx_dev 2>/dev/null; do
  retries=$((retries + 1))
  if [ "$retries" -ge 30 ]; then
    echo "Database failed to become ready after 30s. Check: docker compose logs db"
    exit 1
  fi
  sleep 1
done
echo "       Database is ready."

# Push schema (idempotent)
echo "[3/3] Syncing Drizzle schema..."
cd "$FRONTEND_DIR"
npx drizzle-kit push 2>&1 | tail -3

echo ""
echo "All services up. Starting Next.js dev server..."
echo ""

exec npx next dev
