"""Live metrics API — real-time platform statistics from quorum DB."""

from __future__ import annotations

import json
import logging
import time
from typing import Any

from fastapi import APIRouter

from engine.services.local_inference import inference_router, model_pool

logger = logging.getLogger("engine.routers.metrics")

router = APIRouter()


@router.get("/live")
async def live_metrics() -> dict[str, Any]:
    """Get live platform metrics aggregated from all sources."""
    from engine.services.execution_audit import execution_audit
    from engine.services.orchestrator import orchestrator

    orch_status = orchestrator.get_status()

    # Get quorum stats if available
    quorum_stats: dict[str, Any] = {}
    if execution_audit._enabled and execution_audit._db:
        try:
            quorum_stats = await execution_audit._db.get_run_stats()
        except Exception as e:
            logger.error("Failed to get quorum stats: %s", e)

    return {
        "activeAgents": orch_status.get("total_active_inferences", 0),
        "activeRuns": orch_status.get("active_runs", 0),
        "totalRuns": quorum_stats.get("total_runs", orch_status.get("total_runs", 0)),
        "tasksCompleted": quorum_stats.get("completed", 0),
        "tasksFailed": quorum_stats.get("failed", 0),
        "tokensUsed": quorum_stats.get("total_tokens", 0),
        "avgLatencyMs": quorum_stats.get("avg_duration_ms", 0),
        "avgTokensPerRun": quorum_stats.get("avg_tokens", 0),
        "successRate": quorum_stats.get("success_rate", 0),
        "activeModels": model_pool.active_models,
        "totalActiveInferences": model_pool.total_active,
        "semaphoreAvailable": orch_status.get("semaphore_available", 0),
        "maxConcurrentAgents": orch_status.get("max_concurrent_agents", 10),
    }


@router.get("/history")
async def metrics_history(limit: int = 100) -> dict[str, Any]:
    """Get historical metrics snapshots."""
    from engine.services.execution_audit import execution_audit

    if not execution_audit._enabled or not execution_audit._db:
        return {"snapshots": [], "total": 0}

    try:
        snapshots = await execution_audit._db.get_metrics_history(limit)
        return {"snapshots": snapshots, "total": len(snapshots)}
    except Exception as e:
        logger.error("Failed to get metrics history: %s", e)
        return {"snapshots": [], "total": 0, "error": str(e)}


@router.get("/runs/{run_id}/audit")
async def run_audit_trail(run_id: str) -> dict[str, Any]:
    """Get the full audit trail for a specific run."""
    from engine.services.execution_audit import execution_audit

    if not execution_audit._enabled:
        return {"operations": [], "total": 0}

    try:
        operations = await execution_audit.get_run_operations(run_id)
        return {"operations": operations, "total": len(operations)}
    except Exception as e:
        logger.error("Failed to get audit trail: %s", e)
        return {"operations": [], "total": 0, "error": str(e)}


@router.get("/experts/stats")
async def expert_stats() -> dict[str, Any]:
    """Get aggregated expert performance stats from completed runs."""
    from engine.services.execution_audit import execution_audit

    if not execution_audit._enabled or not execution_audit._db:
        return {"experts": []}

    try:
        pool = execution_audit._db._pool
        rows = await pool.fetch(
            """
            SELECT
                o.metadata->>'stepId' AS step_id,
                COUNT(*) FILTER (WHERE o.operation = 'response' AND o.status = 'ok')
                    AS successful_runs,
                COUNT(*) FILTER (WHERE o.operation = 'step_failed')
                    AS failed_runs,
                COALESCE(SUM(o.tokens_used) FILTER (WHERE o.operation = 'response'), 0)
                    AS total_tokens,
                COALESCE(AVG(o.duration_ms) FILTER (WHERE o.operation = 'response'), 0)::int
                    AS avg_latency_ms,
                COUNT(DISTINCT o.run_id) AS total_runs
            FROM quorum_operations o
            WHERE o.operation IN ('response', 'step_failed')
            GROUP BY o.metadata->>'stepId'
            HAVING COUNT(*) > 0
            """
        )
        experts = [dict(r) for r in rows]
        return {"experts": experts}
    except Exception as e:
        logger.error("Failed to get expert stats: %s", e)
        return {"experts": [], "error": str(e)}


@router.post("/rerun")
async def rerun_step(req: dict[str, Any]) -> dict[str, Any]:
    """Re-run a workflow step with a different model for comparison.

    Request body:
    {
        "run_id": "original-run-id",
        "step_id": "step-to-rerun",
        "engine": "ollama",
        "model": "mistral:7b",
        "temperature": 0.7,
        "max_tokens": 4096
    }
    """
    from engine.services.execution_audit import execution_audit

    run_id = req.get("run_id", "")
    step_id = req.get("step_id", "")
    engine = req.get("engine", "ollama")
    model = req.get("model", "llama3.2:3b")
    temperature = req.get("temperature", 0.7)
    max_tokens = req.get("max_tokens", 4096)

    if not run_id or not step_id:
        return {"error": "run_id and step_id are required"}

    if not execution_audit._enabled or not execution_audit._db:
        return {"error": "Audit trail not available"}

    db_run_id = execution_audit._run_map.get(run_id)
    if not db_run_id:
        return {"error": "Run not found in audit trail"}

    try:
        from engine.services.quorum.types import OpFilter

        ops = await execution_audit._db.list_operations(OpFilter(run_id=db_run_id, phase="execute"))

        # Find the original inference operation for this step
        original_op = None
        for op in ops:
            if op.get("operation") == "response" and op.get("status") == "ok":
                meta = op.get("metadata", {})
                if isinstance(meta, str):
                    meta = json.loads(meta)
                if meta.get("stepId") == step_id or op.get("agent_id", "").endswith(step_id):
                    original_op = op
                    break

        if not original_op:
            return {"error": f"Original operation for step {step_id} not found"}

        # Extract original prompts
        prompt_text = original_op.get("prompt", "")
        parts = prompt_text.split("\n\n[USER]\n", 1)
        system_prompt = parts[0].replace("[SYSTEM]\n", "") if parts else ""
        user_prompt = parts[1] if len(parts) > 1 else ""

        # Re-run with new model
        start = time.monotonic()

        result = await inference_router.chat(
            engine=engine,
            model=model,
            messages=[
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt},
            ],
            temperature=temperature,
            max_tokens=max_tokens,
        )

        duration_ms = int((time.monotonic() - start) * 1000)

        # Log the comparison run
        execution_audit.log_inference(
            run_id,
            f"comparison_{step_id}",
            system_prompt,
            user_prompt,
            result.text,
            result.tokens_used,
            duration_ms,
            model=model,
            engine=engine,
            temperature=temperature,
            max_tokens=max_tokens,
        )

        original_meta = original_op.get("metadata", {})
        if isinstance(original_meta, str):
            original_meta = json.loads(original_meta)

        return {
            "text": result.text,
            "tokensUsed": result.tokens_used,
            "durationMs": duration_ms,
            "model": model,
            "engine": engine,
            "originalModel": original_meta.get("model", "unknown"),
            "originalTokens": original_op.get("tokens_used", 0),
            "originalDurationMs": original_op.get("duration_ms", 0),
        }

    except Exception as e:
        logger.error("Rerun failed: %s", e)
        return {"error": str(e)}
