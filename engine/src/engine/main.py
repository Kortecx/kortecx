from __future__ import annotations

import logging
from contextlib import asynccontextmanager

import uvicorn
from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from engine.config import settings
from engine.core.websocket import ws_manager
from engine.routers import (
    data,
    datasets,
    embeddings,
    inference,
    models,
    orchestrator,
    pipelines,
    synthesis,
    training,
    workflow_logs,
)

logger = logging.getLogger("engine")


@asynccontextmanager
async def lifespan(app: FastAPI):
    """Startup / shutdown lifecycle."""
    logger.info("Kortecx Engine starting — initialising services")

    # Eager-init services so first request isn't slow
    from engine.services.duckdb import duckdb_service
    from engine.services.qdrant import qdrant_service

    await qdrant_service.ensure_collection()
    duckdb_service.ping()

    # Ensure upload directory exists
    from pathlib import Path

    Path(settings.upload_dir).mkdir(parents=True, exist_ok=True)

    logger.info("Services ready")

    yield

    # Shutdown
    logger.info("Kortecx Engine shutting down")
    await ws_manager.disconnect_all()


app = FastAPI(
    title="Kortecx Engine",
    version="0.1.0",
    description="AI/ML engine — data engineering, training, inference, and orchestration",
    lifespan=lifespan,
)

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# ── Routers ──────────────────────────────────────────────────────────────────
app.include_router(data.router, prefix="/api/data", tags=["data"])
app.include_router(datasets.router, prefix="/api/datasets", tags=["datasets"])
app.include_router(synthesis.router, prefix="/api/synthesis", tags=["synthesis"])
app.include_router(models.router, prefix="/api/models", tags=["models"])
app.include_router(inference.router, prefix="/api/inference", tags=["inference"])
app.include_router(training.router, prefix="/api/training", tags=["training"])
app.include_router(embeddings.router, prefix="/api/embeddings", tags=["embeddings"])
app.include_router(pipelines.router, prefix="/api/pipelines", tags=["pipelines"])
app.include_router(orchestrator.router, prefix="/api/orchestrator", tags=["orchestrator"])
app.include_router(workflow_logs.router, prefix="/api/logs", tags=["logs"])

# ── WebSocket ────────────────────────────────────────────────────────────────
app.include_router(ws_manager.router, tags=["websocket"])


@app.get("/health")
async def health():
    return {"status": "ok", "service": "kortecx-engine"}


def run():
    """Entry point for `engine` CLI command."""
    logging.basicConfig(level=logging.DEBUG if settings.debug else logging.INFO)
    uvicorn.run(
        "engine.main:app",
        host=settings.host,
        port=settings.port,
        reload=settings.debug,
    )


if __name__ == "__main__":
    run()
