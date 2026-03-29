"""QuickCheck — platform-aware Q&A powered by the default local model.

All inference goes through ``inference_router`` (never direct Ollama calls)
so retry logic, model-pool tracking, and connection pooling are inherited.
"""

from __future__ import annotations

import json
import logging
import time
import uuid
from typing import Any

import asyncpg
import httpx
import psutil

from engine.config import settings

logger = logging.getLogger("engine.quick_check")


class QuickCheckService:
    """Gathers platform context, prompts the default model, and streams the response."""

    def __init__(self) -> None:
        raw = settings.database_url
        if raw.startswith("postgres://"):
            raw = "postgresql://" + raw[len("postgres://") :]
        self._dsn = raw
        self._pool: asyncpg.Pool | None = None

    async def _ensure_pool(self) -> asyncpg.Pool | None:
        if self._pool is None:
            try:
                self._pool = await asyncpg.create_pool(self._dsn, min_size=1, max_size=3)
            except Exception:
                logger.exception("QuickCheck DB pool creation failed")
        return self._pool

    # ── Context gathering ──────────────────────────────────────────────────

    async def gather_platform_context(self, prompt: str) -> dict[str, Any]:
        """Query NeonDB for platform stats and Qdrant for semantic context."""

        context: dict[str, Any] = {"db": {}, "semantic": []}

        # --- NeonDB platform stats ---
        pool = await self._ensure_pool()
        if pool:
            try:
                row = await pool.fetchrow(
                    """
                    SELECT
                        (SELECT count(*) FROM workflows)              AS workflow_count,
                        (SELECT count(*) FROM experts)                AS expert_count,
                        (SELECT count(*) FROM tasks)                  AS task_count,
                        (SELECT count(*) FROM workflow_runs)          AS run_count,
                        (SELECT count(*) FROM alerts WHERE acknowledged = false) AS active_alerts,
                        (SELECT count(*) FROM workflow_runs WHERE status = 'running') AS running_workflows,
                        (SELECT count(*) FROM expert_runs  WHERE status = 'running')  AS running_experts,
                        (SELECT count(*) FROM datasets)              AS dataset_count
                    """
                )
                if row:
                    context["db"] = {
                        "workflows": row["workflow_count"],
                        "experts": row["expert_count"],
                        "tasks": row["task_count"],
                        "runs": row["run_count"],
                        "active_alerts": row["active_alerts"],
                        "running_workflows": row["running_workflows"],
                        "running_experts": row["running_experts"],
                        "datasets": row["dataset_count"],
                    }
            except Exception as exc:
                logger.warning("DB context query failed: %s", exc)

        # --- Qdrant semantic search ---
        try:
            from engine.services.hf import hf_service
            from engine.services.qdrant import qdrant_service

            vectors = hf_service.text_embedding("sentence-transformers/all-MiniLM-L6-v2", prompt)
            if vectors:
                # Search experts
                prism_results = await qdrant_service.search(
                    vector=vectors[0],
                    limit=5,
                    score_threshold=0.2,
                    collection="kortecx_prisms",
                )
                # Search general embeddings
                general_results = await qdrant_service.search(
                    vector=vectors[0],
                    limit=5,
                    score_threshold=0.2,
                )
                context["semantic"] = [
                    *(
                        {
                            "source": "expert",
                            "name": r["payload"].get("name", ""),
                            "role": r["payload"].get("role", ""),
                            "description": r["payload"].get("description", ""),
                            "score": round(r["score"], 3),
                        }
                        for r in prism_results
                    ),
                    *(
                        {
                            "source": "embedding",
                            "text": (r["payload"].get("text", ""))[:200],
                            "score": round(r["score"], 3),
                        }
                        for r in general_results
                    ),
                ]
        except Exception as exc:
            logger.warning("Qdrant context search failed: %s", exc)

        return context

    # ── System prompt builder ──────────────────────────────────────────────

    def build_system_prompt(self, context: dict[str, Any]) -> str:
        db = context.get("db", {})
        parts = [
            "You are the Kortecx platform assistant.",
            "Help the user understand their platform, data, workflows, and AI agents.",
            "",
            "Current platform state:",
        ]
        if db:
            parts.append(f"  - {db.get('workflows', 0)} workflows, {db.get('experts', 0)} experts, {db.get('datasets', 0)} datasets")
            parts.append(f"  - {db.get('runs', 0)} total runs, {db.get('running_workflows', 0)} workflows running, {db.get('running_experts', 0)} experts running")
            parts.append(f"  - {db.get('tasks', 0)} tasks, {db.get('active_alerts', 0)} active alerts")

        semantic = context.get("semantic", [])
        if semantic:
            experts_ctx = [s for s in semantic if s.get("source") == "expert"]
            if experts_ctx:
                parts.append("")
                parts.append("Relevant experts:")
                for e in experts_ctx[:3]:
                    parts.append(f"  - {e.get('name', '?')} ({e.get('role', '?')}): {e.get('description', '')[:120]}")

            embed_ctx = [s for s in semantic if s.get("source") == "embedding"]
            if embed_ctx:
                parts.append("")
                parts.append("Related knowledge:")
                for e in embed_ctx[:3]:
                    parts.append(f"  - {e.get('text', '')[:150]}")

        parts.append("")
        parts.append("Answer concisely and accurately. Refer to specific platform entities when relevant.")

        return "\n".join(parts)

    # ── Execute (stream + save) ────────────────────────────────────────────

    async def execute(self, check_id: str, prompt: str, conn_id: str | None) -> None:
        """Full quick-check pipeline: health check → model check → context → model → stream → save."""

        from engine.core.websocket import ws_manager
        from engine.services.local_inference import inference_router

        channel = f"quick_check.{check_id}"
        engine_name = settings.default_local_engine
        model_name = settings.default_local_model

        # ── Pre-flight: check inference backend is reachable ────────────────
        try:
            healthy = await inference_router.health_check(engine=engine_name)
        except Exception:
            healthy = False

        if not healthy:
            error_msg = f"{engine_name.capitalize()} is not reachable. Please ensure it is running before using Quick Check."
            logger.warning("QuickCheck %s aborted — %s unreachable", check_id, engine_name)
            await self.save_result(check_id, prompt, None, 0, 0, [], error=error_msg)
            await ws_manager.broadcast(channel, "quick_check.error", {"checkId": check_id, "error": error_msg})
            return

        # ── Pre-flight: check model is available ────────────────────────────
        try:
            models = await inference_router.list_models(engine=engine_name)
            model_names = [m["name"] for m in models]
            # Ollama may list as "llama3.1:8b" or "llama3.1:8b" — check both with and without :latest
            if model_name not in model_names and f"{model_name}:latest" not in model_names:
                error_msg = f"Model '{model_name}' is not available on {engine_name}. Pull it first via the Inference page."
                logger.warning("QuickCheck %s aborted — model '%s' not found", check_id, model_name)
                await self.save_result(check_id, prompt, None, 0, 0, [], error=error_msg)
                await ws_manager.broadcast(channel, "quick_check.error", {"checkId": check_id, "error": error_msg})
                return
        except Exception as exc:
            logger.warning("QuickCheck %s — model list check failed: %s (proceeding anyway)", check_id, exc)

        start = time.time()
        full_response = ""
        tokens_total = 0
        context_sources: list[str] = []

        # Prime CPU measurement (non-blocking baseline)
        psutil.cpu_percent(interval=None)

        try:
            # 1. Gather context
            context = await self.gather_platform_context(prompt)
            if context.get("db"):
                context_sources.append("neondb")
            if context.get("semantic"):
                context_sources.append("qdrant")

            # 2. Build system prompt
            system = self.build_system_prompt(context)

            # 3. Stream via InferenceRouter (uses OllamaService with retries)
            async for token in inference_router.generate_stream(
                engine=engine_name,
                model=model_name,
                prompt=prompt,
                system=system,
                temperature=0.7,
                max_tokens=4096,
            ):
                full_response += token
                tokens_total += 1
                await ws_manager.broadcast(
                    channel,
                    "quick_check.token",
                    {"token": token, "checkId": check_id},
                )

            duration_ms = int((time.time() - start) * 1000)
            cpu_percent = psutil.cpu_percent(interval=None)

            # 4. Save result
            await self.save_result(check_id, prompt, full_response, tokens_total, duration_ms, context_sources)

            # 5. Broadcast completion
            await ws_manager.broadcast(
                channel,
                "quick_check.completed",
                {
                    "checkId": check_id,
                    "response": full_response,
                    "tokensUsed": tokens_total,
                    "durationMs": duration_ms,
                    "contextSources": context_sources,
                    "model": model_name,
                    "engine": engine_name,
                    "cpuPercent": cpu_percent,
                },
            )

        except Exception as exc:
            logger.error("QuickCheck %s failed: %s", check_id, exc)
            duration_ms = int((time.time() - start) * 1000)

            # Humanize common inference errors
            raw = str(exc)
            if isinstance(exc, httpx.ConnectError):
                error_msg = f"Cannot connect to {engine_name}. Ensure it is running on the configured port."
            elif isinstance(exc, (httpx.ReadTimeout, httpx.ConnectTimeout)):
                error_msg = f"{engine_name.capitalize()} timed out generating a response. The model may be overloaded or the prompt too long."
            elif "model" in raw.lower() and "not found" in raw.lower():
                error_msg = f"Model '{model_name}' not found on {engine_name}. Pull it first."
            else:
                error_msg = raw

            await self.save_result(
                check_id,
                prompt,
                full_response or None,
                tokens_total,
                duration_ms,
                context_sources,
                error=error_msg,
            )
            await ws_manager.broadcast(
                channel,
                "quick_check.error",
                {"checkId": check_id, "error": error_msg},
            )

    # ── Persistence ────────────────────────────────────────────────────────

    async def save_result(
        self,
        check_id: str,
        prompt: str,
        response: str | None,
        tokens_used: int,
        duration_ms: int,
        context_sources: list[str],
        *,
        error: str | None = None,
    ) -> None:
        """Persist quick-check result to the quick_checks table."""
        status = "failed" if error else "completed"
        pool = await self._ensure_pool()
        if not pool:
            logger.error("Cannot save quick-check %s — no DB pool", check_id)
            return
        try:
            await pool.execute(
                """
                INSERT INTO quick_checks
                    (id, prompt, response, status, model, engine,
                     tokens_used, duration_ms, context_sources, error_message,
                     completed_at)
                VALUES
                    ($1, $2, $3, $4, $5, $6, $7, $8, $9::jsonb, $10, NOW())
                ON CONFLICT (id) DO UPDATE SET
                    response = EXCLUDED.response,
                    status = EXCLUDED.status,
                    tokens_used = EXCLUDED.tokens_used,
                    duration_ms = EXCLUDED.duration_ms,
                    context_sources = EXCLUDED.context_sources,
                    error_message = EXCLUDED.error_message,
                    completed_at = EXCLUDED.completed_at
                """,
                check_id,
                prompt,
                response or "",
                status,
                settings.default_local_model,
                settings.default_local_engine,
                tokens_used,
                duration_ms,
                json.dumps(context_sources),
                error,
            )
        except Exception as exc:
            logger.error("Failed to save quick-check %s: %s", check_id, exc)

    @staticmethod
    def new_id() -> str:
        return f"qc-{uuid.uuid4().hex[:12]}"


quick_check_service = QuickCheckService()
