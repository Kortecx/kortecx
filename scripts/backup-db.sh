#!/usr/bin/env bash
# ── Kortecx Database Backup Script ──────────────────────────────────────────
# Creates a timestamped pg_dump of the kortecx_dev database.
#
# Usage:
#   ./scripts/backup-db.sh              # backup to ./backups/
#   ./scripts/backup-db.sh /custom/dir  # backup to custom directory
#
# Restore:
#   ./scripts/restore-db.sh backups/kortecx_dev_20260316_120000.sql.gz

set -euo pipefail

BACKUP_DIR="${1:-./backups}"
CONTAINER="kortecx_db"
DB_NAME="kortecx_dev"
DB_USER="kortecx"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
BACKUP_FILE="${BACKUP_DIR}/${DB_NAME}_${TIMESTAMP}.sql.gz"

# Ensure backup directory exists
mkdir -p "$BACKUP_DIR"

# Check container is running
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER}$"; then
  echo "ERROR: Container ${CONTAINER} is not running"
  exit 1
fi

echo "Backing up ${DB_NAME} from ${CONTAINER}..."
docker exec "$CONTAINER" pg_dump -U "$DB_USER" -d "$DB_NAME" --clean --if-exists | gzip > "$BACKUP_FILE"

SIZE=$(du -h "$BACKUP_FILE" | cut -f1)
echo "Backup saved: ${BACKUP_FILE} (${SIZE})"

# Keep only last 10 backups
ls -t "${BACKUP_DIR}/${DB_NAME}"_*.sql.gz 2>/dev/null | tail -n +11 | xargs -r rm -f
echo "Retention: keeping last 10 backups"
