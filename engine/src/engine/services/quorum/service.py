"""Quorum service — top-level orchestrator wiring DB, inference, executor, scheduler, and WS."""

from __future__ import annotations

import asyncio
import logging
from typing import Any
from uuid import UUID

from engine.services.quorum.db import QuorumDB
from engine.services.quorum.executor import PipelineExecutor
from engine.services.quorum.inference import QuorumInferenceClient
from engine.services.quorum.scheduler import Scheduler

logger = logging.getLogger("engine.quorum.service")


class QuorumService:
    """Main Quorum service — wires DB, inference, executor, scheduler, and WS broadcasting.

    Manages WebSocket subscriptions for per-run and global event fan-out.
    """

    def __init__(
        self,
        db_url: str,
        ollama_url: str,
        llamacpp_url: str,
        max_concurrent: int = 4,
        metrics_interval: float = 5.0,
    ) -> None:
        self.db = QuorumDB(db_url)
        self.inference = QuorumInferenceClient(ollama_url, llamacpp_url)
        self.executor = PipelineExecutor(self.inference, self.db, self._broadcast)
        self.scheduler = Scheduler(
            self.executor,
            self.db,
            self._broadcast,
            max_concurrent,
            metrics_interval,
        )
        # run_id -> set of conn_ids; None key = global subscribers
        self._subscribers: dict[UUID | None, set[str]] = {}
        self._lock = asyncio.Lock()

    # ── Lifecycle ────────────────────────────────────────────────────────────

    async def start(self) -> None:
        """Connect to the database, run migrations, and start the scheduler."""
        await self.db.connect()
        await self.scheduler.start()
        logger.info("Quorum service started")

    async def stop(self) -> None:
        """Stop the scheduler and close the database connection pool."""
        await self.scheduler.stop()
        await self.db.close()
        logger.info("Quorum service stopped")

    # ── Subscription Management ──────────────────────────────────────────────

    async def subscribe(self, conn_id: str, run_id: UUID | None = None) -> None:
        """Subscribe a WebSocket connection to events for a specific run or all events."""
        async with self._lock:
            self._subscribers.setdefault(run_id, set()).add(conn_id)

    async def unsubscribe(self, conn_id: str, run_id: UUID | None = None) -> None:
        """Unsubscribe a WebSocket connection from run or global events."""
        async with self._lock:
            if run_id in self._subscribers:
                self._subscribers[run_id].discard(conn_id)
                if not self._subscribers[run_id]:
                    del self._subscribers[run_id]

    async def unsubscribe_all(self, conn_id: str) -> None:
        """Remove a connection from all subscriptions (called on disconnect)."""
        async with self._lock:
            empty_keys: list[UUID | None] = []
            for key, conns in self._subscribers.items():
                conns.discard(conn_id)
                if not conns:
                    empty_keys.append(key)
            for key in empty_keys:
                del self._subscribers[key]

    # ── Broadcasting ─────────────────────────────────────────────────────────

    async def _broadcast(self, run_id: UUID | None, event: str, data: dict[str, Any]) -> None:
        """Fan-out an event to all subscribed WS clients and the quorum channel."""
        from engine.core.websocket import ws_manager

        targets: set[str] = set()
        async with self._lock:
            # Subscribers for this specific run
            if run_id is not None and run_id in self._subscribers:
                targets |= self._subscribers[run_id]
            # Global subscribers (quorum.subscribe.all)
            if None in self._subscribers:
                targets |= self._subscribers[None]

        # Broadcast to the quorum channel (any WS client subscribed via channel)
        channel = f"quorum.{run_id}" if run_id else "quorum"
        await ws_manager.broadcast(channel, event, data)

        # Direct send to explicit subscribers not covered by the channel
        for conn_id in targets:
            try:
                await ws_manager.send_to(conn_id, event, data)
            except Exception:
                pass  # Dead connection, will be cleaned up by ws_manager
