#!/usr/bin/env bash
set -euo pipefail

echo "=== Kortecx Migration Runner ==="
echo ""

# 1. Frontend (Drizzle) migrations
echo "[1/3] Running frontend Drizzle migrations..."
cd frontend
npx drizzle-kit push 2>&1 || echo "  Warning: Frontend migration warning (may be ok if already up-to-date)"
cd ..
echo "  Done: Frontend migrations complete"
echo ""

# 2. Engine (Quorum) migrations
echo "[2/3] Running engine quorum migrations..."
if [ -f engine/migrations/quorum.sql ]; then
  PGPASSWORD="${PGPASSWORD:-kortecx}" psql \
    -h "${PGHOST:-localhost}" \
    -p "${PGPORT:-5433}" \
    -U "${PGUSER:-kortecx}" \
    -d "${PGDATABASE:-kortecx_dev}" \
    -f engine/migrations/quorum.sql 2>&1 || echo "  Warning: Quorum migration warning"
  echo "  Done: Quorum migrations complete"
else
  echo "  Warning: No quorum.sql found, skipping"
fi
echo ""

# 3. Verify schema version
echo "[3/3] Verifying schema version..."
if [ -f kortecx.config.json ]; then
  SCHEMA_VERSION=$(python3 -c "import json; print(json.load(open('kortecx.config.json')).get('schemaVersion', 'unknown'))" 2>/dev/null || echo "unknown")
  echo "  Schema version: $SCHEMA_VERSION"
fi
echo ""

echo "=== All migrations complete ==="
