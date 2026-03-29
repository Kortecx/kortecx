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
    experts,
    inference,
    lineage,
    mcp,
    metrics,
    models,
    orchestrator,
    pipelines,
    plans,
    providers,
    search,
    synthesis,
    training,
    workflow_logs,
)
from engine.routers.mlflow_router import router as mlflow_router
from engine.routers.quick_check import router as _qc_router

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
    (Path(__file__).resolve().parents[2] / "mcp").mkdir(parents=True, exist_ok=True)
    (Path(__file__).resolve().parents[2] / "mcp" / "prompts").mkdir(parents=True, exist_ok=True)
    (Path(__file__).resolve().parents[2] / "mcp_scripts").mkdir(parents=True, exist_ok=True)
    (Path(__file__).resolve().parents[2] / "plans" / "LIVE").mkdir(parents=True, exist_ok=True)
    (Path(__file__).resolve().parents[2] / "plans" / "FREEZE").mkdir(parents=True, exist_ok=True)

    # Load expert definitions
    from engine.services.expert_manager import expert_manager

    expert_manager.load_all()
    logger.info("Loaded %d experts", len(expert_manager._cache))

    # Auto-embed all loaded experts into Qdrant for graph similarity
    from engine.routers.experts import _embed_agent
    from engine.services.hf import hf_service

    if hf_service.has_token:
        logger.info("HF_TOKEN set — using HuggingFace Inference API for embeddings")
    else:
        logger.info("HF_TOKEN not set — will use local sentence-transformers if available")

    embedded_count = 0
    for exp in expert_manager._cache.values():
        src = exp.get("_source", "local")
        try:
            await _embed_agent(exp, source=src)
            embedded_count += 1
        except Exception:
            logger.warning("Failed to embed expert %s on startup", exp.get("id"))
    if embedded_count:
        logger.info("Embedded %d experts into Qdrant on startup", embedded_count)

    # Quorum multi-agent orchestration engine
    from engine.routers.quorum import init_quorum

    quorum_svc = await init_quorum()
    logger.info("Quorum service initialized")

    # Wire execution audit to quorum DB
    from engine.services.execution_audit import execution_audit

    execution_audit.set_db(quorum_svc.db)

    logger.info("Services ready")

    yield

    # Shutdown
    logger.info("Kortecx Engine shutting down")
    await quorum_svc.stop()
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
app.include_router(mlflow_router, prefix="/api/mlflow", tags=["mlflow"])
app.include_router(mcp.router, prefix="/api/mcp", tags=["mcp"])
app.include_router(experts.router, prefix="/api/agents/engine", tags=["experts"])
app.include_router(metrics.router, prefix="/api/metrics", tags=["metrics"])
app.include_router(lineage.router, prefix="/api/lineage", tags=["lineage"])
app.include_router(providers.router, prefix="/api/providers", tags=["providers"])
app.include_router(search.router, prefix="/api/search", tags=["search"])
app.include_router(plans.router, prefix="/api/plans", tags=["plans"])

app.include_router(_qc_router, prefix="/api/quick-check", tags=["quick-check"])

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
