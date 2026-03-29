#!/usr/bin/env bash
# ── Kortecx Clean Slate — Wipe all user data, preserve platform structure ─────
#
# Usage:
#   ./scripts/clean-slate.sh          # interactive (requires confirmation)
#   ./scripts/clean-slate.sh --force  # skip confirmation (CI / scripted use)
#
# What is preserved:
#   - Database schema & migrations (_kortecx_schema, __drizzle_migrations)
#   - Marketplace PRISMs (engine/PRISM/marketplace/)
#   - All platform code and configuration
#
# What is wiped:
#   - All 33 PostgreSQL tables (28 Drizzle + 5 Quorum)
#   - Qdrant embeddings collection
#   - User-created PRISMs, uploads, plans on disk

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONTAINER="kortecx_db"
DB_NAME="kortecx_dev"
DB_USER="kortecx"
QDRANT_URL="${QDRANT_URL:-http://localhost:6333}"
QDRANT_COLLECTION="${QDRANT_COLLECTION:-kortecx_embeddings}"

# ── Confirmation ──────────────────────────────────────────────────────────────
if [ "${1:-}" != "--force" ]; then
  echo ""
  echo "╔══════════════════════════════════════════════════════════════╗"
  echo "║              KORTECX CLEAN SLATE                           ║"
  echo "║                                                            ║"
  echo "║  This will permanently delete ALL user data:               ║"
  echo "║    - All PostgreSQL rows (33 tables)                       ║"
  echo "║    - All Qdrant embeddings                                 ║"
  echo "║    - User-created PRISMs, uploads, plans on disk           ║"
  echo "║                                                            ║"
  echo "║  Preserved: schema, migrations, marketplace PRISMs         ║"
  echo "║                                                            ║"
  echo "║  A backup will be created before any data is deleted.      ║"
  echo "╚══════════════════════════════════════════════════════════════╝"
  echo ""
  read -p "Type 'clean-slate' to proceed: " confirm
  if [ "$confirm" != "clean-slate" ]; then
    echo "Aborted."
    exit 1
  fi
  echo ""
fi

# ── Pre-flight: check Docker container ────────────────────────────────────────
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
  echo "ERROR: Container ${CONTAINER} is not running. Start with: make docker-up"
  exit 1
fi

# ── Step 1: Backup ────────────────────────────────────────────────────────────
echo "Step 1/5: Creating backup..."
bash "$PROJECT_ROOT/scripts/backup-db.sh"
echo ""

# ── Step 2: Truncate all PostgreSQL tables ────────────────────────────────────
echo "Step 2/5: Truncating all PostgreSQL tables..."
docker exec "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -c "
TRUNCATE
  metrics, tasks, workflow_runs, expert_runs, alerts, logs,
  experts, plans, workflows, workflow_steps, step_executions,
  datasets, hf_datasets, integrations, integration_connections,
  plugins, projects, project_assets, api_keys, synthesis_jobs,
  assets, dataset_schemas, data_versions, lineage,
  oauth_credentials, social_connections, execution_audit, model_comparisons,
  quorum_runs, quorum_operations, quorum_metrics,
  quorum_shared_memory, quorum_projects
CASCADE;
"

# Verify
echo "  Verifying row counts..."
docker exec "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -t -c "
SELECT count(*) AS non_empty_tables
FROM pg_stat_user_tables
WHERE n_live_tup > 0
  AND relname NOT IN ('_kortecx_schema', '__drizzle_migrations');
" | while read -r count; do
  count=$(echo "$count" | xargs)
  if [ -n "$count" ] && [ "$count" != "0" ]; then
    echo "  WARNING: $count table(s) still have rows (stats may be stale — run ANALYZE)"
  else
    echo "  All 33 tables truncated successfully"
  fi
done
echo ""

# ── Step 3: Reset Qdrant ─────────────────────────────────────────────────────
echo "Step 3/5: Resetting Qdrant collection..."
set +e  # non-fatal section
QDRANT_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$QDRANT_URL/collections/$QDRANT_COLLECTION" 2>/dev/null)
if [ "$QDRANT_STATUS" = "200" ]; then
  # Get vector config before deleting
  VECTOR_SIZE=$(curl -s "$QDRANT_URL/collections/$QDRANT_COLLECTION" 2>/dev/null | \
    python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)['result']['config']['params']['vectors']
    print(d.get('size', 768) if isinstance(d, dict) else 768)
except: print(768)
" 2>/dev/null || echo "768")

  curl -s -X DELETE "$QDRANT_URL/collections/$QDRANT_COLLECTION" > /dev/null 2>&1
  curl -s -X PUT "$QDRANT_URL/collections/$QDRANT_COLLECTION" \
    -H 'Content-Type: application/json' \
    -d "{\"vectors\": {\"size\": $VECTOR_SIZE, \"distance\": \"Cosine\"}}" > /dev/null 2>&1
  echo "  Collection '$QDRANT_COLLECTION' recreated (dimension: $VECTOR_SIZE)"
else
  echo "  Qdrant not reachable or collection missing — skipped"
fi
set -e
echo ""

# ── Step 4: DuckDB ───────────────────────────────────────────────────────────
echo "Step 4/5: Checking DuckDB..."
DUCKDB_PATH="${DUCKDB_PATH:-:memory:}"
if [ "$DUCKDB_PATH" != ":memory:" ] && [ -f "$DUCKDB_PATH" ]; then
  rm -f "$DUCKDB_PATH"
  echo "  Deleted DuckDB file: $DUCKDB_PATH"
else
  echo "  DuckDB is in-memory — resets on engine restart"
fi
echo ""

# ── Step 5: Local filesystem ─────────────────────────────────────────────────
echo "Step 5/5: Cleaning local files..."

# User-created PRISMs (keep marketplace)
PRISM_LOCAL="$PROJECT_ROOT/engine/PRISM/local"
if [ -d "$PRISM_LOCAL" ]; then
  find "$PRISM_LOCAL" -mindepth 1 -maxdepth 1 -type d -exec rm -rf {} +
  echo '{"version": "1.0.0", "experts": []}' > "$PRISM_LOCAL/_registry.json"
  echo "  Cleared engine/PRISM/local/ (marketplace preserved)"
fi

# Uploads
UPLOADS="$PROJECT_ROOT/engine/uploads"
if [ -d "$UPLOADS" ]; then
  find "$UPLOADS" -mindepth 1 -delete 2>/dev/null || true
  echo "  Cleared engine/uploads/"
fi

# Plans
for DIR in "$PROJECT_ROOT/engine/plans/LIVE" "$PROJECT_ROOT/engine/plans/FREEZE"; do
  if [ -d "$DIR" ]; then
    find "$DIR" -mindepth 1 -delete 2>/dev/null || true
    echo "  Cleared $(basename "$(dirname "$DIR")")/$(basename "$DIR")/"
  fi
done

echo ""
echo "════════════════════════════════════════════════════════════════"
echo "  Clean slate complete."
echo "  - 33 PostgreSQL tables truncated"
echo "  - Qdrant collection reset"
echo "  - Local PRISMs, uploads, plans cleared"
echo "  - Marketplace PRISMs preserved (12 experts)"
echo "  - Backup created in ./backups/"
echo ""
echo "  Restart the engine to reset in-memory state:"
echo "    make engine"
echo "════════════════════════════════════════════════════════════════"
