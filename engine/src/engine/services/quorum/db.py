"""Quorum database layer — high-performance async PostgreSQL via asyncpg."""

from __future__ import annotations

import asyncio
import json
import logging
from datetime import UTC, datetime
from typing import Any
from uuid import UUID

import asyncpg

from engine.services.quorum.types import (
    MetricsSnapshot,
    Operation,
    OpFilter,
    RunFilter,
    RunRequest,
)

logger = logging.getLogger("engine.quorum.db")

# ── Embedded migration SQL ───────────────────────────────────────────────────

_MIGRATION_SQL = """
CREATE TABLE IF NOT EXISTS quorum_runs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project           TEXT NOT NULL,
    task              TEXT NOT NULL,
    system_prompt     TEXT DEFAULT '',
    backend           TEXT NOT NULL,
    model             TEXT,
    workers           INTEGER NOT NULL,
    status            TEXT NOT NULL DEFAULT 'queued',
    config            JSONB,
    started_at        TIMESTAMPTZ,
    finished_at       TIMESTAMPTZ,
    total_tokens      BIGINT DEFAULT 0,
    total_duration_ms BIGINT DEFAULT 0,
    decompose_ms      BIGINT DEFAULT 0,
    execute_ms        BIGINT DEFAULT 0,
    synthesize_ms     BIGINT DEFAULT 0,
    final_output      TEXT,
    error             TEXT,
    workers_succeeded INTEGER DEFAULT 0,
    workers_failed    INTEGER DEFAULT 0,
    workers_recovered INTEGER DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_operations (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id        UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
    agent_id      TEXT NOT NULL,
    phase         TEXT NOT NULL,
    operation     TEXT NOT NULL,
    prompt        TEXT,
    response      TEXT,
    tokens_used   BIGINT DEFAULT 0,
    duration_ms   BIGINT DEFAULT 0,
    status        TEXT NOT NULL DEFAULT 'ok',
    error         TEXT,
    metadata      JSONB,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_metrics (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cpu_usage       REAL NOT NULL,
    memory_usage_mb REAL DEFAULT 0,
    active_runs     INTEGER NOT NULL,
    queued_runs     INTEGER NOT NULL,
    tokens_per_sec  DOUBLE PRECISION DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_shared_memory (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id     UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
    phase      TEXT NOT NULL,
    memory     JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_projects (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT NOT NULL UNIQUE,
    config     JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quorum_ops_run ON quorum_operations(run_id);
CREATE INDEX IF NOT EXISTS idx_quorum_ops_agent ON quorum_operations(agent_id);
CREATE INDEX IF NOT EXISTS idx_quorum_ops_phase ON quorum_operations(run_id, phase);
CREATE INDEX IF NOT EXISTS idx_quorum_runs_project ON quorum_runs(project);
CREATE INDEX IF NOT EXISTS idx_quorum_runs_status ON quorum_runs(status);
CREATE INDEX IF NOT EXISTS idx_quorum_metrics_time ON quorum_metrics(created_at);
CREATE INDEX IF NOT EXISTS idx_quorum_memory_run ON quorum_shared_memory(run_id);
"""


class QuorumDB:
    """Async PostgreSQL operations for the quorum engine using asyncpg connection pool."""

    def __init__(self, dsn: str) -> None:
        self._dsn = dsn
        self._pool: asyncpg.Pool | None = None
        self._ops_queue: asyncio.Queue[Operation] = asyncio.Queue(maxsize=10000)
        self._writer_task: asyncio.Task | None = None

    # ── Lifecycle ────────────────────────────────────────────────────────────

    async def connect(self) -> None:
        """Create connection pool, run migrations, start background writer."""
        self._pool = await asyncpg.create_pool(self._dsn, min_size=2, max_size=10)
        await self.migrate()
        self._writer_task = asyncio.create_task(self._background_writer())
        logger.info("Quorum DB connected and migrated")

    async def close(self) -> None:
        """Drain the operation queue and close the connection pool."""
        if self._writer_task:
            self._writer_task.cancel()
            try:
                await self._writer_task
            except asyncio.CancelledError:
                pass
        # Drain remaining operations
        while not self._ops_queue.empty():
            try:
                op = self._ops_queue.get_nowait()
                await self.insert_operation(op)
            except Exception:
                break
        if self._pool:
            await self._pool.close()
        logger.info("Quorum DB closed")

    async def migrate(self) -> None:
        """Execute the embedded migration SQL (idempotent via IF NOT EXISTS)."""
        assert self._pool is not None, "Pool not initialized"
        async with self._pool.acquire() as conn:
            await conn.execute(_MIGRATION_SQL)
        logger.info("Quorum DB migration complete")

    # ── Fire-and-forget operation logging ────────────────────────────────────

    def log_operation(self, op: Operation) -> None:
        """Enqueue an operation for background persistence. Non-blocking."""
        try:
            self._ops_queue.put_nowait(op)
        except asyncio.QueueFull:
            logger.warning("Quorum ops queue full, dropping operation for agent=%s", op.agent_id)

    async def _background_writer(self) -> None:
        """Continuously drain the operation queue and persist to PostgreSQL."""
        batch: list[Operation] = []
        while True:
            try:
                # Wait for the first item
                op = await self._ops_queue.get()
                batch.append(op)
                # Drain any additional queued items (up to 50 per batch)
                while len(batch) < 50:
                    try:
                        batch.append(self._ops_queue.get_nowait())
                    except asyncio.QueueEmpty:
                        break
                # Batch insert
                await self._batch_insert_operations(batch)
                batch.clear()
            except asyncio.CancelledError:
                raise
            except Exception as e:
                logger.error("Background writer failed: %s", e)
                batch.clear()

    async def _batch_insert_operations(self, ops: list[Operation]) -> None:
        """Batch insert multiple operations in a single transaction."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.executemany(
                """
                INSERT INTO quorum_operations
                    (run_id, agent_id, phase, operation, prompt, response,
                     tokens_used, duration_ms, status, error, metadata)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                """,
                [
                    (
                        op.run_id,
                        op.agent_id,
                        op.phase,
                        op.operation,
                        op.prompt,
                        op.response,
                        op.tokens_used,
                        op.duration_ms,
                        op.status,
                        op.error,
                        json.dumps(op.metadata) if op.metadata else None,
                    )
                    for op in ops
                ],
            )

    # ── Runs CRUD ────────────────────────────────────────────────────────────

    async def create_run(self, run: RunRequest, run_id: UUID) -> dict[str, Any]:
        """Insert a new run and return the created row as a dict."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow(
                """
                INSERT INTO quorum_runs
                    (id, project, task, system_prompt, backend, model, workers, status, config)
                VALUES ($1, $2, $3, $4, $5, $6, $7, 'queued', $8)
                RETURNING *
                """,
                run_id,
                run.project,
                run.task,
                run.system_prompt,
                run.backend,
                run.model,
                run.workers,
                json.dumps(run.config) if run.config else None,
            )
            return dict(row) if row else {}

    async def get_run(self, run_id: UUID) -> dict[str, Any] | None:
        """Fetch a single run by ID."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow("SELECT * FROM quorum_runs WHERE id = $1", run_id)
            return dict(row) if row else None

    async def list_runs(self, f: RunFilter) -> list[dict[str, Any]]:
        """List runs with optional filtering, ordered by creation time descending."""
        assert self._pool is not None
        conditions: list[str] = []
        params: list[Any] = []
        idx = 1

        if f.project:
            conditions.append(f"project = ${idx}")
            params.append(f.project)
            idx += 1
        if f.status:
            conditions.append(f"status = ${idx}")
            params.append(f.status)
            idx += 1

        where = f"WHERE {' AND '.join(conditions)}" if conditions else ""
        params.extend([f.limit, f.offset])

        query = f"SELECT * FROM quorum_runs {where} ORDER BY created_at DESC LIMIT ${idx} OFFSET ${idx + 1}"

        async with self._pool.acquire() as conn:
            rows = await conn.fetch(query, *params)
            return [dict(r) for r in rows]

    async def update_run(self, run_id: UUID, **fields: Any) -> None:
        """Update arbitrary fields on a run. Automatically sets updated_at."""
        assert self._pool is not None
        if not fields:
            return

        fields["updated_at"] = datetime.now(UTC)
        set_clauses: list[str] = []
        params: list[Any] = []

        for idx, (key, value) in enumerate(fields.items(), start=1):
            set_clauses.append(f"{key} = ${idx}")
            params.append(value)

        params.append(run_id)
        query = f"UPDATE quorum_runs SET {', '.join(set_clauses)} WHERE id = ${len(params)}"

        async with self._pool.acquire() as conn:
            await conn.execute(query, *params)

    async def delete_run(self, run_id: UUID) -> None:
        """Delete a run and all associated operations/memory (cascaded)."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute("DELETE FROM quorum_runs WHERE id = $1", run_id)

    # ── Operations ───────────────────────────────────────────────────────────

    async def insert_operation(self, op: Operation) -> None:
        """Insert a single operation record."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO quorum_operations
                    (run_id, agent_id, phase, operation, prompt, response,
                     tokens_used, duration_ms, status, error, metadata)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                """,
                op.run_id,
                op.agent_id,
                op.phase,
                op.operation,
                op.prompt,
                op.response,
                op.tokens_used,
                op.duration_ms,
                op.status,
                op.error,
                json.dumps(op.metadata) if op.metadata else None,
            )

    async def list_operations(self, f: OpFilter) -> list[dict[str, Any]]:
        """List operations with optional filtering."""
        assert self._pool is not None
        conditions: list[str] = []
        params: list[Any] = []
        idx = 1

        if f.run_id:
            conditions.append(f"run_id = ${idx}")
            params.append(f.run_id)
            idx += 1
        if f.agent_id:
            conditions.append(f"agent_id = ${idx}")
            params.append(f.agent_id)
            idx += 1
        if f.phase:
            conditions.append(f"phase = ${idx}")
            params.append(f.phase)
            idx += 1
        if f.operation:
            conditions.append(f"operation = ${idx}")
            params.append(f.operation)
            idx += 1

        where = f"WHERE {' AND '.join(conditions)}" if conditions else ""
        params.extend([f.limit, f.offset])

        query = f"SELECT * FROM quorum_operations {where} ORDER BY created_at ASC LIMIT ${idx} OFFSET ${idx + 1}"

        async with self._pool.acquire() as conn:
            rows = await conn.fetch(query, *params)
            return [dict(r) for r in rows]

    # ── Metrics ──────────────────────────────────────────────────────────────

    async def insert_metrics(self, m: MetricsSnapshot) -> None:
        """Store a system metrics snapshot."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO quorum_metrics
                    (cpu_usage, memory_usage_mb, active_runs, queued_runs, tokens_per_sec)
                VALUES ($1, $2, $3, $4, $5)
                """,
                m.cpu_usage,
                m.memory_usage_mb,
                m.active_runs,
                m.queued_runs,
                m.tokens_per_sec,
            )

    async def get_metrics_history(self, limit: int = 100) -> list[dict[str, Any]]:
        """Retrieve recent metrics snapshots ordered by time descending."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            rows = await conn.fetch(
                "SELECT * FROM quorum_metrics ORDER BY created_at DESC LIMIT $1",
                limit,
            )
            return [dict(r) for r in rows]

    # ── Shared Memory ────────────────────────────────────────────────────────

    async def save_shared_memory(self, run_id: UUID, phase: str, memory: dict[str, Any]) -> None:
        """Persist a phase memory snapshot for inter-phase communication."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO quorum_shared_memory (run_id, phase, memory)
                VALUES ($1, $2, $3)
                """,
                run_id,
                phase,
                json.dumps(memory),
            )

    async def get_shared_memory(self, run_id: UUID) -> list[dict[str, Any]]:
        """Retrieve all shared memory snapshots for a run, ordered by creation time."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            rows = await conn.fetch(
                "SELECT * FROM quorum_shared_memory WHERE run_id = $1 ORDER BY created_at ASC",
                run_id,
            )
            return [dict(r) for r in rows]

    # ── Projects ─────────────────────────────────────────────────────────────

    async def upsert_project(self, name: str, config: dict[str, Any]) -> None:
        """Create or update a project configuration."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute(
                """
                INSERT INTO quorum_projects (name, config)
                VALUES ($1, $2)
                ON CONFLICT (name) DO UPDATE
                    SET config = $2, updated_at = NOW()
                """,
                name,
                json.dumps(config),
            )

    async def get_project(self, name: str) -> dict[str, Any] | None:
        """Fetch a project by name."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            row = await conn.fetchrow("SELECT * FROM quorum_projects WHERE name = $1", name)
            return dict(row) if row else None

    async def list_projects(self) -> list[dict[str, Any]]:
        """List all projects ordered by name."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            rows = await conn.fetch("SELECT * FROM quorum_projects ORDER BY name ASC")
            return [dict(r) for r in rows]

    async def delete_project(self, name: str) -> None:
        """Delete a project configuration."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            await conn.execute("DELETE FROM quorum_projects WHERE name = $1", name)

    # ── Aggregations ─────────────────────────────────────────────────────────

    async def get_run_stats(self, project: str | None = None) -> dict[str, Any]:
        """Aggregate statistics across runs, optionally filtered by project."""
        assert self._pool is not None
        _stats_query = """
                SELECT
                    COUNT(*) AS total_runs,
                    COUNT(*) FILTER (WHERE status = 'complete') AS completed,
                    COUNT(*) FILTER (WHERE status = 'failed') AS failed,
                    COUNT(*) FILTER (WHERE status = 'running') AS running,
                    COUNT(*) FILTER (WHERE status = 'queued') AS queued,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens,
                    COALESCE(AVG(total_duration_ms), 0)::bigint AS avg_duration_ms,
                    COALESCE(AVG(total_tokens), 0)::bigint AS avg_tokens,
                    CASE WHEN COUNT(*) > 0
                         THEN ROUND(COUNT(*) FILTER (WHERE status = 'complete')::numeric
                                    / COUNT(*)::numeric * 100, 1)
                         ELSE 0
                    END AS success_rate,
                    COALESCE(SUM(workers_succeeded), 0) AS total_workers_succeeded,
                    COALESCE(SUM(workers_failed), 0) AS total_workers_failed,
                    COALESCE(SUM(workers_recovered), 0) AS total_workers_recovered
                FROM quorum_runs
        """
        if project:
            query = _stats_query + " WHERE project = $1"
            params: list[Any] = [project]
        else:
            query = _stats_query
            params = []

        async with self._pool.acquire() as conn:
            row = await conn.fetchrow(query, *params)
            return dict(row) if row else {}

    async def get_token_usage_by_agent(self, run_id: UUID) -> list[dict[str, Any]]:
        """Aggregate token usage per agent within a run."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            rows = await conn.fetch(
                """
                SELECT agent_id,
                       SUM(tokens_used) AS total_tokens,
                       COUNT(*) AS operation_count,
                       SUM(duration_ms) AS total_duration_ms
                FROM quorum_operations
                WHERE run_id = $1
                GROUP BY agent_id
                ORDER BY total_tokens DESC
                """,
                run_id,
            )
            return [dict(r) for r in rows]

    async def get_phase_timeline(self, run_id: UUID) -> list[dict[str, Any]]:
        """Get the timeline of phases for a run, aggregated by phase."""
        assert self._pool is not None
        async with self._pool.acquire() as conn:
            rows = await conn.fetch(
                """
                SELECT phase,
                       MIN(created_at) AS started_at,
                       MAX(created_at) AS ended_at,
                       COUNT(*) AS operation_count,
                       SUM(tokens_used) AS total_tokens,
                       SUM(duration_ms) AS total_duration_ms
                FROM quorum_operations
                WHERE run_id = $1
                GROUP BY phase
                ORDER BY MIN(created_at) ASC
                """,
                run_id,
            )
            return [dict(r) for r in rows]
