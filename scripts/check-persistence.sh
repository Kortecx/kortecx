#!/usr/bin/env bash
# ── Kortecx Persistence Check ──────────────────────────────────────────────
# Verifies that Docker volumes are properly configured and data persists
# across container recreation. Safe to run at any time.
#
# Usage: ./scripts/check-persistence.sh

set -euo pipefail

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m'

pass() { echo -e "${GREEN}PASS${NC} $1"; }
fail() { echo -e "${RED}FAIL${NC} $1"; FAILURES=$((FAILURES + 1)); }
warn() { echo -e "${YELLOW}WARN${NC} $1"; }

FAILURES=0

echo "═══════════════════════════════════════════════"
echo " Kortecx Persistence Check"
echo "═══════════════════════════════════════════════"
echo ""

# 1. Check named volumes exist
echo "── Docker Volumes ──"
if docker volume inspect kortecx_pgdata > /dev/null 2>&1; then
  pass "pgdata volume exists"
else
  fail "pgdata volume NOT found — run 'docker compose up db'"
fi

if docker volume inspect kortecx_qdrant_data > /dev/null 2>&1; then
  pass "qdrant_data volume exists"
else
  fail "qdrant_data volume NOT found — run 'docker compose up qdrant'"
fi

# 2. Check containers are running
echo ""
echo "── Containers ──"
if docker ps --format '{{.Names}}' | grep -q "kortecx_db"; then
  pass "PostgreSQL container running"
else
  fail "PostgreSQL container NOT running"
fi

if docker ps --format '{{.Names}}' | grep -q "kortecx_qdrant"; then
  pass "Qdrant container running"
else
  warn "Qdrant container not running (optional)"
fi

# 3. Check PostgreSQL has data
echo ""
echo "── PostgreSQL Data ──"
if docker ps --format '{{.Names}}' | grep -q "kortecx_db"; then
  TABLE_COUNT=$(docker exec kortecx_db psql -U kortecx -d kortecx_dev -t -c "SELECT count(*) FROM information_schema.tables WHERE table_schema='public';" 2>/dev/null | tr -d ' ')
  if [ "$TABLE_COUNT" -gt 0 ] 2>/dev/null; then
    pass "Database has ${TABLE_COUNT} tables"
  else
    warn "Database has 0 tables — run 'cd frontend && npx drizzle-kit push'"
  fi

  EXPERT_COUNT=$(docker exec kortecx_db psql -U kortecx -d kortecx_dev -t -c "SELECT count(*) FROM experts;" 2>/dev/null | tr -d ' ')
  echo "     Experts: ${EXPERT_COUNT}"

  DS_COUNT=$(docker exec kortecx_db psql -U kortecx -d kortecx_dev -t -c "SELECT count(*) FROM datasets;" 2>/dev/null | tr -d ' ')
  echo "     Datasets: ${DS_COUNT}"

  KEY_COUNT=$(docker exec kortecx_db psql -U kortecx -d kortecx_dev -t -c "SELECT count(*) FROM api_keys WHERE status='active';" 2>/dev/null | tr -d ' ')
  echo "     Active API keys: ${KEY_COUNT}"
fi

# 4. Check volume mount points
echo ""
echo "── Volume Mounts ──"
PG_MOUNT=$(docker inspect kortecx_db 2>/dev/null | grep -A5 '"Destination": "/var/lib/postgresql/data"' | head -1)
if [ -n "$PG_MOUNT" ]; then
  pass "PostgreSQL data mounted at /var/lib/postgresql/data"
else
  fail "PostgreSQL data NOT properly mounted"
fi

# 5. Check backups directory
echo ""
echo "── Backups ──"
BACKUP_COUNT=$(ls -1 backups/*.sql.gz 2>/dev/null | wc -l | tr -d ' ')
if [ "$BACKUP_COUNT" -gt 0 ]; then
  pass "${BACKUP_COUNT} backup(s) found in ./backups/"
  LATEST=$(ls -t backups/*.sql.gz 2>/dev/null | head -1)
  echo "     Latest: ${LATEST}"
else
  warn "No backups found — run './scripts/backup-db.sh'"
fi

# Summary
echo ""
echo "═══════════════════════════════════════════════"
if [ "$FAILURES" -gt 0 ]; then
  echo -e "${RED}${FAILURES} check(s) failed${NC}"
  exit 1
else
  echo -e "${GREEN}All persistence checks passed${NC}"
fi
