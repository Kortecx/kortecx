.PHONY: start stop frontend engine docker-up docker-down db-push db-seed install clean

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

# ── Go Client ────────────────────────────────────────────────────────────────
go-test:
	cd go-client && go test ./...

go-vet:
	cd go-client && go vet ./...

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

# ── Clean ────────────────────────────────────────────────────────────────────
clean:
	rm -rf frontend/.next frontend/node_modules engine/.venv
