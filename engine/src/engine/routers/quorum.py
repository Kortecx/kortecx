"""Quorum WebSocket event handlers — registered in the WebSocketManager."""

from __future__ import annotations

import logging
from typing import Any
from uuid import UUID

from engine.config import settings
from engine.services.quorum.errors import ValidationError
from engine.services.quorum.service import QuorumService
from engine.services.quorum.types import OpFilter, RunFilter, RunRequest

logger = logging.getLogger("engine.quorum")

# ── Module-level singleton (initialized at startup) ──────────────────────────

quorum_handler: QuorumHandler | None = None


async def init_quorum() -> QuorumService:
    """Initialize the QuorumService and install the global event handler.

    Called once during app lifespan startup.
    """
    global quorum_handler  # noqa: PLW0603

    svc = QuorumService(
        db_url=settings.database_url,
        ollama_url=settings.ollama_url,
        llamacpp_url=settings.llamacpp_url,
        max_concurrent=settings.quorum_max_concurrent,
        metrics_interval=settings.quorum_metrics_interval,
    )
    await svc.start()

    quorum_handler = QuorumHandler(svc)
    logger.info("Quorum handler initialized")
    return svc


class QuorumHandler:
    """Handles all quorum.* WebSocket events."""

    def __init__(self, service: QuorumService) -> None:
        self.svc = service

    async def handle(self, conn_id: str, event: str, data: dict[str, Any]) -> dict[str, Any] | None:
        """Route a quorum event to the appropriate handler. Returns response data or None."""
        handlers: dict[str, Any] = {
            "quorum.run.submit": self._handle_submit,
            "quorum.run.cancel": self._handle_cancel,
            "quorum.run.status": self._handle_status,
            "quorum.run.get": self._handle_get_run,
            "quorum.run.list": self._handle_list_runs,
            "quorum.run.delete": self._handle_delete_run,
            "quorum.run.stats": self._handle_run_stats,
            "quorum.run.token_usage": self._handle_token_usage,
            "quorum.run.timeline": self._handle_timeline,
            "quorum.run.memory": self._handle_shared_memory,
            "quorum.config.get": self._handle_get_config,
            "quorum.config.update": self._handle_update_config,
            "quorum.models.list": self._handle_list_models,
            "quorum.models.pull": self._handle_pull_model,
            "quorum.models.health": self._handle_health,
            "quorum.metrics.get": self._handle_get_metrics,
            "quorum.metrics.history": self._handle_metrics_history,
            "quorum.operations.list": self._handle_list_operations,
            "quorum.projects.list": self._handle_list_projects,
            "quorum.projects.get": self._handle_get_project,
            "quorum.projects.upsert": self._handle_upsert_project,
            "quorum.projects.delete": self._handle_delete_project,
            "quorum.subscribe": self._handle_subscribe,
            "quorum.subscribe.all": self._handle_subscribe_all,
            "quorum.unsubscribe": self._handle_unsubscribe,
        }

        handler = handlers.get(event)
        if handler:
            return await handler(conn_id, data)
        logger.warning("Unknown quorum event: %s", event)
        return None

    # ── Run Management ───────────────────────────────────────────────────────

    async def _handle_submit(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        task = data.get("task", "")
        if not task:
            raise ValidationError("Task is required")

        request = RunRequest(
            project=data.get("project", "default"),
            task=task,
            model=data.get("model", settings.default_local_model),
            backend=data.get("backend", settings.default_local_engine),
            workers=data.get("workers", settings.quorum_default_workers),
            system_prompt=data.get("prompt", data.get("system_prompt", "")),
            temperature=data.get("temperature", 0.7),
            max_tokens=data.get("max_tokens", 2048),
            retries=data.get("retries", settings.quorum_default_retries),
            config=data.get("config"),
        )

        # Auto-subscribe sender to this run's events
        run_id = await self.svc.scheduler.submit(request)
        await self.svc.subscribe(conn_id, run_id)

        return {"run_id": str(run_id), "status": "queued"}

    async def _handle_cancel(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        cancelled = await self.svc.scheduler.cancel(run_id)
        return {"run_id": str(run_id), "cancelled": cancelled}

    async def _handle_status(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        status = self.svc.scheduler.status(run_id)
        if status is None:
            # Try from DB
            row = await self.svc.db.get_run(run_id)
            if row:
                return _row_to_status(row)
            return {"error": "Run not found", "run_id": str(run_id)}
        return status

    async def _handle_get_run(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        row = await self.svc.db.get_run(run_id)
        if row:
            return _row_to_status(row)
        return {"error": "Run not found", "run_id": str(run_id)}

    async def _handle_list_runs(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        f = RunFilter(
            project=data.get("project"),
            status=data.get("status"),
            limit=data.get("limit", 50),
            offset=data.get("offset", 0),
        )
        rows = await self.svc.db.list_runs(f)
        return {"runs": [_row_to_status(r) for r in rows], "count": len(rows)}

    async def _handle_delete_run(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        await self.svc.db.delete_run(run_id)
        self.svc.scheduler._registry.pop(run_id, None)
        return {"run_id": str(run_id), "deleted": True}

    async def _handle_run_stats(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        project = data.get("project")
        stats = await self.svc.db.get_run_stats(project)
        return _serialize_row(stats)

    async def _handle_token_usage(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        usage = await self.svc.db.get_token_usage_by_agent(run_id)
        return {"run_id": str(run_id), "usage": [_serialize_row(u) for u in usage]}

    async def _handle_timeline(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        timeline = await self.svc.db.get_phase_timeline(run_id)
        return {"run_id": str(run_id), "phases": [_serialize_row(t) for t in timeline]}

    async def _handle_shared_memory(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        memory = await self.svc.db.get_shared_memory(run_id)
        return {"run_id": str(run_id), "memory": [_serialize_row(m) for m in memory]}

    # ── Configuration ────────────────────────────────────────────────────────

    async def _handle_get_config(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        return {
            "max_concurrent": settings.quorum_max_concurrent,
            "metrics_interval": settings.quorum_metrics_interval,
            "default_workers": settings.quorum_default_workers,
            "default_retries": settings.quorum_default_retries,
            "default_model": settings.default_local_model,
            "default_backend": settings.default_local_engine,
            "ollama_url": settings.ollama_url,
            "llamacpp_url": settings.llamacpp_url,
        }

    async def _handle_update_config(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        # Runtime config updates (in-memory only, doesn't persist to env)
        if "max_concurrent" in data:
            self.svc.scheduler._max_concurrent = int(data["max_concurrent"])
        if "metrics_interval" in data:
            self.svc.scheduler._metrics_interval = float(data["metrics_interval"])
        return {"updated": True}

    # ── Models ───────────────────────────────────────────────────────────────

    async def _handle_list_models(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        backend = data.get("backend", settings.default_local_engine)
        models = await self.svc.inference.list_models(backend)
        return {"backend": backend, "models": models}

    async def _handle_pull_model(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        model = data.get("model", "")
        backend = data.get("backend", "ollama")
        if not model:
            raise ValidationError("Model name is required")
        await self.svc.inference.pull_model(model, backend)
        return {"model": model, "pulled": True}

    async def _handle_health(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        backend = data.get("backend", settings.default_local_engine)
        healthy = await self.svc.inference.health(backend)
        return {"backend": backend, "healthy": healthy}

    # ── Metrics ──────────────────────────────────────────────────────────────

    async def _handle_get_metrics(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        return self.svc.scheduler.metrics()

    async def _handle_metrics_history(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        limit = data.get("limit", 100)
        history = await self.svc.db.get_metrics_history(limit)
        return {"metrics": [_serialize_row(m) for m in history]}

    # ── Operations ───────────────────────────────────────────────────────────

    async def _handle_list_operations(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        f = OpFilter(
            run_id=UUID(data["run_id"]) if data.get("run_id") else None,
            agent_id=data.get("agent_id"),
            phase=data.get("phase"),
            operation=data.get("operation"),
            limit=data.get("limit", 100),
            offset=data.get("offset", 0),
        )
        ops = await self.svc.db.list_operations(f)
        return {"operations": [_serialize_row(o) for o in ops], "count": len(ops)}

    # ── Projects ─────────────────────────────────────────────────────────────

    async def _handle_list_projects(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        projects = await self.svc.db.list_projects()
        return {"projects": [_serialize_row(p) for p in projects]}

    async def _handle_get_project(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        name = data.get("name", "")
        if not name:
            raise ValidationError("Project name is required")
        project = await self.svc.db.get_project(name)
        if project:
            return _serialize_row(project)
        return {"error": "Project not found", "name": name}

    async def _handle_upsert_project(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        name = data.get("name", "")
        config = data.get("config", {})
        if not name:
            raise ValidationError("Project name is required")
        await self.svc.db.upsert_project(name, config)
        return {"name": name, "upserted": True}

    async def _handle_delete_project(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        name = data.get("name", "")
        if not name:
            raise ValidationError("Project name is required")
        await self.svc.db.delete_project(name)
        return {"name": name, "deleted": True}

    # ── Subscriptions ────────────────────────────────────────────────────────

    async def _handle_subscribe(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        await self.svc.subscribe(conn_id, run_id)
        return {"subscribed": str(run_id)}

    async def _handle_subscribe_all(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        await self.svc.subscribe(conn_id, None)
        return {"subscribed": "all"}

    async def _handle_unsubscribe(self, conn_id: str, data: dict[str, Any]) -> dict[str, Any]:
        run_id = UUID(data["run_id"])
        await self.svc.unsubscribe(conn_id, run_id)
        return {"unsubscribed": str(run_id)}


# ── Helpers ──────────────────────────────────────────────────────────────────


def _row_to_status(row: dict[str, Any]) -> dict[str, Any]:
    """Convert a DB row dict to a JSON-safe status dict."""
    return _serialize_row(row)


def _serialize_row(row: dict[str, Any]) -> dict[str, Any]:
    """Convert a DB row dict to a JSON-serializable dict."""
    result: dict[str, Any] = {}
    for key, value in row.items():
        if isinstance(value, UUID):
            result[key] = str(value)
        elif hasattr(value, "isoformat"):
            result[key] = value.isoformat()
        else:
            result[key] = value
    return result
