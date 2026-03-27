"""Expert sync service — bridges engine filesystem experts with shared PostgreSQL."""

from __future__ import annotations

import json
import logging
from datetime import UTC, datetime
from typing import Any

import asyncpg

from engine.config import settings

logger = logging.getLogger("engine.expert_sync")

# ── Field mapping: engine expert.json (camelCase) → PostgreSQL columns (snake_case) ──

_ENGINE_TO_DB: dict[str, str] = {
    "id": "id",
    "name": "name",
    "description": "description",
    "role": "role",
    "version": "version",
    "modelSource": "model_source",
    "localModelConfig": "local_model_config",
    "temperature": "temperature",
    "maxTokens": "max_tokens",
    "category": "category",
    "complexityLevel": "complexity_level",
    "tags": "tags",
    "isPublic": "is_public",
    "createdAt": "created_at",
    "updatedAt": "updated_at",
}

_DB_TO_ENGINE: dict[str, str] = {v: k for k, v in _ENGINE_TO_DB.items()}


def _to_db_row(expert: dict[str, Any]) -> dict[str, Any]:
    """Convert an engine expert dict to a flat dict matching PostgreSQL columns."""
    now = datetime.now(UTC)
    system_prompt = expert.get("systemPrompt", "")

    # Extract model info from localModelConfig when model_source == "local"
    local_cfg = expert.get("localModelConfig") or {}
    engine_name = local_cfg.get("engine", "ollama") if local_cfg else "ollama"
    model_name_val = local_cfg.get("modelName") or local_cfg.get("model", "")

    row: dict[str, Any] = {
        "id": expert["id"],
        "name": expert.get("name", ""),
        "description": expert.get("description", ""),
        "role": expert.get("role", "general"),
        "status": expert.get("status", "idle"),
        "version": expert.get("version", "1.0.0"),
        "model_id": f"{engine_name}:{model_name_val}" if model_name_val else "local:unknown",
        "model_name": model_name_val or "unknown",
        "provider_id": engine_name,
        "provider_name": engine_name,
        "model_source": expert.get("modelSource", "local"),
        "local_model_config": json.dumps(local_cfg) if local_cfg else None,
        "system_prompt": system_prompt or None,
        "temperature": float(expert.get("temperature", 0.7)),
        "max_tokens": int(expert.get("maxTokens", 4096)),
        "category": expert.get("category", "custom"),
        "complexity_level": int(expert.get("complexityLevel", 3)),
        "tags": expert.get("tags") or None,
        "is_public": bool(expert.get("isPublic", False)),
        "is_finetuned": bool(expert.get("isFinetuned", False)),
        "replica_count": int(expert.get("replicaCount", 1)),
    }

    # Parse timestamps or use now
    for ts_field, db_col in [("createdAt", "created_at"), ("updatedAt", "updated_at")]:
        raw = expert.get(ts_field)
        if isinstance(raw, str):
            try:
                row[db_col] = datetime.fromisoformat(raw.replace("Z", "+00:00"))
            except ValueError:
                row[db_col] = now
        elif isinstance(raw, datetime):
            row[db_col] = raw
        else:
            row[db_col] = now

    return row


def _from_db_row(row: dict[str, Any]) -> dict[str, Any]:
    """Convert a PostgreSQL row dict back to the engine expert format."""
    expert: dict[str, Any] = {
        "id": row["id"],
        "name": row.get("name", ""),
        "description": row.get("description", ""),
        "role": row.get("role", "general"),
        "status": row.get("status", "idle"),
        "version": row.get("version", "1.0.0"),
        "modelSource": row.get("model_source", "local"),
        "temperature": float(row["temperature"]) if row.get("temperature") is not None else 0.7,
        "maxTokens": row.get("max_tokens", 4096),
        "category": row.get("category", "custom"),
        "complexityLevel": row.get("complexity_level", 3),
        "tags": row.get("tags") or [],
        "isPublic": bool(row.get("is_public", False)),
        "isFinetuned": bool(row.get("is_finetuned", False)),
        "replicaCount": row.get("replica_count", 1),
        "modelId": row.get("model_id", ""),
        "modelName": row.get("model_name", ""),
        "providerId": row.get("provider_id", ""),
        "providerName": row.get("provider_name", ""),
        "systemPrompt": row.get("system_prompt", ""),
        "totalRuns": row.get("total_runs", 0),
        "successRate": float(row["success_rate"]) if row.get("success_rate") is not None else 0.0,
        "avgLatencyMs": row.get("avg_latency_ms", 0),
        "avgCostPerRun": float(row["avg_cost_per_run"]) if row.get("avg_cost_per_run") is not None else 0.0,
        "rating": float(row["rating"]) if row.get("rating") is not None else 0.0,
    }

    # Parse local_model_config from JSONB
    lmc = row.get("local_model_config")
    if isinstance(lmc, str):
        try:
            expert["localModelConfig"] = json.loads(lmc)
        except (json.JSONDecodeError, TypeError):
            expert["localModelConfig"] = {}
    elif isinstance(lmc, dict):
        expert["localModelConfig"] = lmc
    else:
        expert["localModelConfig"] = {}

    # Timestamps
    for db_col, engine_key in [("created_at", "createdAt"), ("updated_at", "updatedAt")]:
        val = row.get(db_col)
        if isinstance(val, datetime):
            expert[engine_key] = val.isoformat()
        elif val is not None:
            expert[engine_key] = str(val)

    return expert


# ── Upsert SQL ────────────────────────────────────────────────────────────────

_UPSERT_SQL = """
INSERT INTO experts (
    id, name, description, role, status, version,
    model_id, model_name, provider_id, provider_name,
    model_source, local_model_config,
    system_prompt, temperature, max_tokens,
    category, complexity_level,
    tags, is_public, is_finetuned, replica_count,
    created_at, updated_at
) VALUES (
    $1, $2, $3, $4, $5, $6,
    $7, $8, $9, $10,
    $11, $12,
    $13, $14, $15,
    $16, $17,
    $18, $19, $20, $21,
    $22, $23
)
ON CONFLICT (id) DO UPDATE SET
    name              = EXCLUDED.name,
    description       = EXCLUDED.description,
    role              = EXCLUDED.role,
    version           = EXCLUDED.version,
    model_id          = EXCLUDED.model_id,
    model_name        = EXCLUDED.model_name,
    provider_id       = EXCLUDED.provider_id,
    provider_name     = EXCLUDED.provider_name,
    model_source      = EXCLUDED.model_source,
    local_model_config = EXCLUDED.local_model_config,
    system_prompt     = EXCLUDED.system_prompt,
    temperature       = EXCLUDED.temperature,
    max_tokens        = EXCLUDED.max_tokens,
    category          = EXCLUDED.category,
    complexity_level  = EXCLUDED.complexity_level,
    tags              = EXCLUDED.tags,
    is_public         = EXCLUDED.is_public,
    is_finetuned      = EXCLUDED.is_finetuned,
    replica_count     = EXCLUDED.replica_count,
    updated_at        = NOW()
"""

_STATS_SQL = """
UPDATE experts SET
    total_runs      = COALESCE($2, total_runs),
    success_rate    = COALESCE($3, success_rate),
    avg_latency_ms  = COALESCE($4, avg_latency_ms),
    avg_cost_per_run = COALESCE($5, avg_cost_per_run),
    rating          = COALESCE($6, rating),
    updated_at      = NOW()
WHERE id = $1
"""


class ExpertSyncService:
    """Syncs engine filesystem experts with the shared PostgreSQL experts table."""

    def __init__(self, dsn: str | None = None) -> None:
        raw = dsn or settings.database_url
        # asyncpg requires postgresql:// not postgres:// — normalise
        if raw.startswith("postgres://"):
            raw = "postgresql://" + raw[len("postgres://") :]
        self._dsn = raw
        self._pool: asyncpg.Pool | None = None

    # ── Lifecycle ────────────────────────────────────────────────────────────

    async def connect(self) -> None:
        """Create the asyncpg connection pool."""
        if self._pool is not None:
            return
        try:
            self._pool = await asyncpg.create_pool(self._dsn, min_size=1, max_size=5)
            logger.info("ExpertSyncService connected to PostgreSQL")
        except Exception:
            logger.exception("ExpertSyncService failed to connect — sync will be unavailable")
            self._pool = None

    async def close(self) -> None:
        """Close the connection pool."""
        if self._pool:
            await self._pool.close()
            self._pool = None
            logger.info("ExpertSyncService connection closed")

    @property
    def available(self) -> bool:
        """Return True if the DB pool is ready."""
        return self._pool is not None

    # ── Core sync methods ────────────────────────────────────────────────────

    async def sync_to_db(self, expert_data: dict[str, Any]) -> None:
        """Upsert a single engine expert into the PostgreSQL experts table.

        Non-blocking: logs a warning on failure but never raises.
        """
        if not self.available:
            logger.warning("sync_to_db skipped — DB pool not available")
            return
        try:
            row = _to_db_row(expert_data)
            assert self._pool is not None
            async with self._pool.acquire() as conn:
                await conn.execute(
                    _UPSERT_SQL,
                    row["id"],
                    row["name"],
                    row["description"],
                    row["role"],
                    row["status"],
                    row["version"],
                    row["model_id"],
                    row["model_name"],
                    row["provider_id"],
                    row["provider_name"],
                    row["model_source"],
                    row["local_model_config"],
                    row["system_prompt"],
                    row["temperature"],
                    row["max_tokens"],
                    row["category"],
                    row["complexity_level"],
                    row["tags"],
                    row["is_public"],
                    row["is_finetuned"],
                    row["replica_count"],
                    row["created_at"],
                    row["updated_at"],
                )
            logger.info("Synced expert %s to PostgreSQL", row["id"])
        except Exception:
            logger.exception("Failed to sync expert %s to DB", expert_data.get("id"))

    async def sync_from_db(self) -> list[dict[str, Any]]:
        """Read all experts from the PostgreSQL experts table.

        Returns an empty list on failure (non-blocking).
        """
        if not self.available:
            logger.warning("sync_from_db skipped — DB pool not available")
            return []
        try:
            assert self._pool is not None
            async with self._pool.acquire() as conn:
                rows = await conn.fetch("SELECT * FROM experts ORDER BY name ASC")
            return [_from_db_row(dict(r)) for r in rows]
        except Exception:
            logger.exception("Failed to read experts from DB")
            return []

    async def sync_stats_to_db(self, expert_id: str, stats: dict[str, Any]) -> None:
        """Update performance stats for an expert in PostgreSQL.

        Accepted stats keys: total_runs, success_rate, avg_latency_ms,
        avg_cost_per_run, rating.

        Non-blocking: logs a warning on failure but never raises.
        """
        if not self.available:
            logger.warning("sync_stats_to_db skipped — DB pool not available")
            return
        try:
            assert self._pool is not None
            async with self._pool.acquire() as conn:
                await conn.execute(
                    _STATS_SQL,
                    expert_id,
                    stats.get("total_runs"),
                    float(stats["success_rate"]) if "success_rate" in stats else None,
                    stats.get("avg_latency_ms"),
                    float(stats["avg_cost_per_run"]) if "avg_cost_per_run" in stats else None,
                    float(stats["rating"]) if "rating" in stats else None,
                )
            logger.info("Synced stats for expert %s", expert_id)
        except Exception:
            logger.exception("Failed to sync stats for expert %s", expert_id)

    async def delete_from_db(self, expert_id: str) -> None:
        """Delete an expert record from PostgreSQL.

        Non-blocking: logs a warning on failure but never raises.
        """
        if not self.available:
            logger.warning("delete_from_db skipped — DB pool not available")
            return
        try:
            assert self._pool is not None
            async with self._pool.acquire() as conn:
                await conn.execute("DELETE FROM experts WHERE id = $1", expert_id)
            logger.info("Deleted expert %s from PostgreSQL", expert_id)
        except Exception:
            logger.exception("Failed to delete expert %s from DB", expert_id)


# ── Singleton ────────────────────────────────────────────────────────────────

expert_sync = ExpertSyncService()
