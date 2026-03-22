"""Quorum scheduler — FIFO run queue with concurrent capacity management and metrics."""

from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Any
from uuid import UUID, uuid4

import psutil

from engine.services.quorum.db import QuorumDB
from engine.services.quorum.executor import PipelineExecutor
from engine.services.quorum.types import (
    MetricsSnapshot,
    RunFilter,
    RunRequest,
)

logger = logging.getLogger("engine.quorum.scheduler")

Broadcaster = Callable[[UUID | None, str, dict[str, Any]], Awaitable[None]]


class Scheduler:
    """FIFO run queue with concurrent capacity management and periodic metrics emission.

    Dequeues runs when capacity is available and dispatches them to the PipelineExecutor.
    Supports cancellation, status tracking, and real-time metrics broadcasting.
    """

    def __init__(
        self,
        executor: PipelineExecutor,
        db: QuorumDB,
        broadcaster: Broadcaster,
        max_concurrent: int = 4,
        metrics_interval: float = 5.0,
    ) -> None:
        self._executor = executor
        self._db = db
        self._broadcast = broadcaster
        self._max_concurrent = max_concurrent
        self._metrics_interval = metrics_interval

        self._queue: asyncio.Queue[tuple[UUID, RunRequest]] = asyncio.Queue()
        self._active: dict[UUID, asyncio.Task[None]] = {}
        self._cancels: dict[UUID, asyncio.Event] = {}
        self._registry: dict[UUID, dict[str, Any]] = {}

        self._total_completed: int = 0
        self._total_tokens: int = 0
        self._total_duration_ms: int = 0

        self._dispatcher_task: asyncio.Task[None] | None = None
        self._metrics_task: asyncio.Task[None] | None = None
        self._running: bool = False

    # ── Lifecycle ────────────────────────────────────────────────────────────

    async def start(self) -> None:
        """Start the dispatcher and metrics collection loops."""
        self._running = True
        self._dispatcher_task = asyncio.create_task(self._dispatcher(), name="quorum-dispatcher")
        self._metrics_task = asyncio.create_task(self._metrics_loop(), name="quorum-metrics")
        logger.info("Quorum scheduler started (max_concurrent=%d)", self._max_concurrent)

    async def stop(self) -> None:
        """Cancel all active runs and background tasks, then drain."""
        self._running = False
        for run_id, task in list(self._active.items()):
            cancel_evt = self._cancels.get(run_id)
            if cancel_evt:
                cancel_evt.set()
            task.cancel()
        if self._dispatcher_task:
            self._dispatcher_task.cancel()
            try:
                await self._dispatcher_task
            except asyncio.CancelledError:
                pass
        if self._metrics_task:
            self._metrics_task.cancel()
            try:
                await self._metrics_task
            except asyncio.CancelledError:
                pass
        logger.info("Quorum scheduler stopped")

    # ── Public API ───────────────────────────────────────────────────────────

    async def submit(self, request: RunRequest) -> UUID:
        """Submit a new run to the queue. Returns the generated run ID."""
        run_id = uuid4()

        self._registry[run_id] = {
            "id": str(run_id),
            "project": request.project,
            "task": request.task,
            "status": "queued",
            "workers": request.workers,
            "phase": "",
            "backend": request.backend,
            "model": request.model,
            "created_at": datetime.now(UTC).isoformat(),
            "started_at": None,
            "total_tokens": 0,
            "total_duration_ms": 0,
        }

        await self._db.create_run(request, run_id)
        await self._queue.put((run_id, request))

        await self._broadcast(
            run_id,
            "quorum.run.queued",
            {
                "run_id": str(run_id),
                "project": request.project,
                "task": request.task,
                "position": self._queue.qsize(),
            },
        )

        logger.info("Run %s queued (project=%s, workers=%d)", run_id, request.project, request.workers)
        return run_id

    async def cancel(self, run_id: UUID) -> bool:
        """Cancel a running or queued run. Returns True if cancellation was initiated."""
        if run_id in self._cancels:
            self._cancels[run_id].set()
            if run_id in self._active:
                self._active[run_id].cancel()
            await self._db.update_run(run_id, status="cancelled", finished_at=datetime.now(UTC))
            if run_id in self._registry:
                self._registry[run_id]["status"] = "cancelled"

            await self._broadcast(
                run_id,
                "quorum.run.cancelled",
                {"run_id": str(run_id)},
            )
            logger.info("Run %s cancelled", run_id)
            return True
        return False

    def status(self, run_id: UUID) -> dict[str, Any] | None:
        """Get the in-memory status of a run."""
        return self._registry.get(run_id)

    def list_runs(self, f: RunFilter | None = None) -> list[dict[str, Any]]:
        """List runs from the in-memory registry with optional filtering."""
        runs = list(self._registry.values())
        if f:
            if f.project:
                runs = [r for r in runs if r.get("project") == f.project]
            if f.status:
                runs = [r for r in runs if r.get("status") == f.status]
            runs = runs[f.offset : f.offset + f.limit]
        return runs

    def metrics(self) -> dict[str, Any]:
        """Return current scheduler metrics."""
        avg_duration = int(self._total_duration_ms / max(self._total_completed, 1))
        return {
            "active_runs": len(self._active),
            "queued_runs": self._queue.qsize(),
            "max_concurrent": self._max_concurrent,
            "total_runs_completed": self._total_completed,
            "total_tokens_used": self._total_tokens,
            "avg_run_duration_ms": avg_duration,
        }

    # ── Dispatcher ───────────────────────────────────────────────────────────

    async def _dispatcher(self) -> None:
        """Dequeue runs when capacity is available and dispatch to executor."""
        while self._running:
            if len(self._active) < self._max_concurrent:
                try:
                    run_id, request = await asyncio.wait_for(self._queue.get(), timeout=1.0)
                except TimeoutError:
                    continue
                except asyncio.CancelledError:
                    raise

                cancel_event = asyncio.Event()
                self._cancels[run_id] = cancel_event
                task = asyncio.create_task(
                    self._run_pipeline(run_id, request, cancel_event),
                    name=f"quorum-run-{run_id}",
                )
                self._active[run_id] = task
            else:
                await asyncio.sleep(0.5)

    async def _run_pipeline(self, run_id: UUID, request: RunRequest, cancel_event: asyncio.Event) -> None:
        """Execute a single pipeline run with full lifecycle management."""
        try:
            now = datetime.now(UTC)
            await self._db.update_run(run_id, status="running", started_at=now)
            if run_id in self._registry:
                self._registry[run_id]["status"] = "running"
                self._registry[run_id]["started_at"] = now.isoformat()

            await self._broadcast(
                run_id,
                "quorum.run.started",
                {
                    "run_id": str(run_id),
                    "project": request.project,
                    "workers": request.workers,
                    "backend": request.backend,
                    "model": request.model,
                },
            )

            result = await self._executor.execute(run_id, request, cancel_event)

            await self._db.update_run(
                run_id,
                status="complete",
                finished_at=datetime.now(UTC),
                total_tokens=result.total_tokens,
                total_duration_ms=result.total_duration_ms,
                decompose_ms=result.decompose_ms,
                execute_ms=result.execute_ms,
                synthesize_ms=result.synthesize_ms,
                final_output=result.final_output,
                workers_succeeded=result.workers_succeeded,
                workers_failed=result.workers_failed,
                workers_recovered=result.workers_recovered,
            )

            if run_id in self._registry:
                self._registry[run_id]["status"] = "complete"
                self._registry[run_id]["total_tokens"] = result.total_tokens
                self._registry[run_id]["total_duration_ms"] = result.total_duration_ms

            self._total_completed += 1
            self._total_tokens += result.total_tokens
            self._total_duration_ms += result.total_duration_ms

            await self._broadcast(
                run_id,
                "quorum.run.complete",
                {
                    "run_id": str(run_id),
                    "total_tokens": result.total_tokens,
                    "total_duration_ms": result.total_duration_ms,
                    "decompose_ms": result.decompose_ms,
                    "execute_ms": result.execute_ms,
                    "synthesize_ms": result.synthesize_ms,
                    "final_output": result.final_output,
                    "workers_succeeded": result.workers_succeeded,
                    "workers_failed": result.workers_failed,
                    "workers_recovered": result.workers_recovered,
                },
            )

            logger.info(
                "Run %s complete — tokens=%d, duration=%dms",
                run_id,
                result.total_tokens,
                result.total_duration_ms,
            )

        except asyncio.CancelledError:
            await self._db.update_run(run_id, status="cancelled", error="Cancelled", finished_at=datetime.now(UTC))
            if run_id in self._registry:
                self._registry[run_id]["status"] = "cancelled"
            logger.info("Run %s cancelled", run_id)

        except Exception as e:
            error_msg = str(e)
            await self._db.update_run(run_id, status="failed", error=error_msg, finished_at=datetime.now(UTC))
            if run_id in self._registry:
                self._registry[run_id]["status"] = "failed"

            await self._broadcast(
                run_id,
                "quorum.run.failed",
                {"run_id": str(run_id), "error": error_msg, "phase": "execution"},
            )
            logger.error("Run %s failed: %s", run_id, error_msg)

        finally:
            self._active.pop(run_id, None)
            self._cancels.pop(run_id, None)

    # ── Metrics Loop ─────────────────────────────────────────────────────────

    async def _metrics_loop(self) -> None:
        """Emit system metrics snapshots at regular intervals."""
        while self._running:
            await asyncio.sleep(self._metrics_interval)
            try:
                cpu = psutil.cpu_percent(interval=None)
                mem = psutil.virtual_memory().used / (1024 * 1024)
                tokens_per_sec = (self._total_tokens / (self._total_duration_ms / 1000)) if self._total_duration_ms > 0 else 0.0

                snapshot = MetricsSnapshot(
                    cpu_usage=cpu,
                    memory_usage_mb=mem,
                    active_runs=len(self._active),
                    queued_runs=self._queue.qsize(),
                    tokens_per_sec=tokens_per_sec,
                )
                await self._db.insert_metrics(snapshot)

                avg_duration = int(self._total_duration_ms / max(self._total_completed, 1))
                await self._broadcast(
                    None,
                    "quorum.metrics.snapshot",
                    {
                        "active_runs": len(self._active),
                        "queued_runs": self._queue.qsize(),
                        "max_concurrent": self._max_concurrent,
                        "cpu_usage": cpu,
                        "memory_usage_mb": round(mem, 1),
                        "tokens_per_sec": round(tokens_per_sec, 2),
                        "total_runs_completed": self._total_completed,
                        "total_tokens_used": self._total_tokens,
                        "avg_run_duration_ms": avg_duration,
                    },
                )
            except asyncio.CancelledError:
                raise
            except Exception as e:
                logger.error("Metrics collection failed: %s", e)
