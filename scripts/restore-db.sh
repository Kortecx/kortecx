#!/usr/bin/env bash
# ── Kortecx Database Restore Script ─────────────────────────────────────────
# Restores from a pg_dump backup file.
#
# Usage:
#   ./scripts/restore-db.sh backups/kortecx_dev_20260316_120000.sql.gz
#
# WARNING: This will overwrite all data in the target database!

set -euo pipefail

BACKUP_FILE="${1:?Usage: $0 <backup_file.sql.gz>}"
CONTAINER="kortecx_db"
DB_NAME="kortecx_dev"
DB_USER="kortecx"

if [ ! -f "$BACKUP_FILE" ]; then
  echo "ERROR: Backup file not found: ${BACKUP_FILE}"
  exit 1
fi

if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
  echo "ERROR: Container ${CONTAINER} is not running"
  exit 1
fi

echo "WARNING: This will overwrite ALL data in ${DB_NAME}!"
read -p "Continue? (y/N) " -r
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
  echo "Aborted."
  exit 0
fi

echo "Restoring ${DB_NAME} from ${BACKUP_FILE}..."
gunzip -c "$BACKUP_FILE" | docker exec -i "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" --quiet

echo "Restore complete."
docker exec "$CONTAINER" psql -U "$DB_USER" -d "$DB_NAME" -c "SELECT count(*) as tables FROM information_schema.tables WHERE table_schema='public';"
