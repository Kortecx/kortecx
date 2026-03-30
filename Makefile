.PHONY: start stop frontend engine docker-up docker-down db-push db-seed install clean clean-slate backup restore check-persistence

# ── Full stack ────────────────────────────────────────────────────────────────
start:
	bash start.sh

stop:
	docker compose down
	-pkill -f "uvicorn engine.main:app" 2>/dev/null

# ── Docker ────────────────────────────────────────────────────────────────────
docker-up:
	docker compose up -d

docker-down:
	docker compose down

docker-reset:
	@echo "WARNING: This deletes ALL data volumes! Run 'make backup' first."
	@read -p "Are you sure? (y/N) " confirm && [ "$$confirm" = "y" ] || exit 1
	docker compose down -v && docker compose up -d

# ── Frontend (Next.js) ───────────────────────────────────────────────────────
frontend:
	cd frontend && npx next dev

frontend-build:
	cd frontend && npx next build

frontend-install:
	cd frontend && npm install

# ── Engine (Python FastAPI) ──────────────────────────────────────────────────
engine:
	cd engine && uv run uvicorn engine.main:app --host 0.0.0.0 --port 8000 --reload

engine-install:
	cd engine && uv sync

# ── Database ─────────────────────────────────────────────────────────────────
db-push:
	cd frontend && npx drizzle-kit push

db-seed:
	psql $${DATABASE_URL} -f ./frontend/scripts/seed.sql

db-studio:
	cd frontend && npx drizzle-kit studio

# ── Install everything ───────────────────────────────────────────────────────
install: frontend-install engine-install
	@echo "All dependencies installed"

# ── Backup & Persistence ──────────────────────────────────────────────────
backup:
	bash scripts/backup-db.sh

backup-logs:
	docker logs -f kortecx_db_backup

restore:
	@echo "Usage: make restore FILE=backups/kortecx_dev_YYYYMMDD_HHMMSS.sql.gz"
	@test -n "$(FILE)" && bash scripts/restore-db.sh $(FILE) || echo "Specify FILE=<path>"

check-persistence:
	bash scripts/check-persistence.sh

# ── Clean ────────────────────────────────────────────────────────────────────
clean:
	rm -rf frontend/.next frontend/node_modules engine/.venv

clean-slate:
	@echo "=== CLEAN SLATE: Wipe ALL user data (schema + marketplace preserved) ==="
	bash scripts/clean-slate.sh
