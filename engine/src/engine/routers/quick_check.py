"""Quick Check REST API — history listing and management."""

from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

router = APIRouter()


class SubmitRequest(BaseModel):
    """Request body for REST-based quick check submission."""

    checkId: str | None = None
    prompt: str


async def _get_pool():
    """Lazy access to the QuickCheck service's asyncpg pool."""
    from engine.services.quick_check import quick_check_service

    return await quick_check_service._ensure_pool()


@router.post("/submit")
async def submit_quick_check(req: SubmitRequest) -> dict[str, Any]:
    """Submit a quick check via REST (fallback when WebSocket is unavailable)."""
    import asyncio

    from engine.services.quick_check import quick_check_service

    prompt = req.prompt.strip()
    check_id = req.checkId or quick_check_service.new_id()

    if not prompt:
        return {"error": "Prompt is required"}

    asyncio.create_task(quick_check_service.execute(check_id, prompt, conn_id=None))
    return {"checkId": check_id, "status": "accepted"}


@router.get("")
async def list_quick_checks(limit: int = 50) -> dict[str, Any]:
    """List recent quick checks, newest first."""
    pool = await _get_pool()
    if not pool:
        return {"checks": [], "total": 0, "error": "DB unavailable"}

    rows = await pool.fetch(
        "SELECT id, prompt, response, status, model, engine, "
        "tokens_used, duration_ms, context_sources, error_message, "
        "created_at, completed_at "
        "FROM quick_checks ORDER BY created_at DESC LIMIT $1",
        limit,
    )

    checks = [
        {
            "id": r["id"],
            "prompt": r["prompt"],
            "response": r["response"],
            "status": r["status"],
            "model": r["model"],
            "engine": r["engine"],
            "tokensUsed": r["tokens_used"],
            "durationMs": r["duration_ms"],
            "contextSources": r["context_sources"],
            "errorMessage": r["error_message"],
            "createdAt": r["created_at"].isoformat() if r["created_at"] else None,
            "completedAt": r["completed_at"].isoformat() if r["completed_at"] else None,
        }
        for r in rows
    ]
    return {"checks": checks, "total": len(checks)}


@router.get("/{check_id}")
async def get_quick_check(check_id: str) -> dict[str, Any]:
    """Get a single quick check by ID."""
    pool = await _get_pool()
    if not pool:
        return {"error": "DB unavailable"}

    r = await pool.fetchrow(
        "SELECT id, prompt, response, status, model, engine, "
        "tokens_used, duration_ms, context_sources, error_message, "
        "created_at, completed_at "
        "FROM quick_checks WHERE id = $1",
        check_id,
    )

    if not r:
        return {"error": "Not found"}

    return {
        "id": r["id"],
        "prompt": r["prompt"],
        "response": r["response"],
        "status": r["status"],
        "model": r["model"],
        "engine": r["engine"],
        "tokensUsed": r["tokens_used"],
        "durationMs": r["duration_ms"],
        "contextSources": r["context_sources"],
        "errorMessage": r["error_message"],
        "createdAt": r["created_at"].isoformat() if r["created_at"] else None,
        "completedAt": r["completed_at"].isoformat() if r["completed_at"] else None,
    }


@router.delete("/{check_id}")
async def delete_quick_check(check_id: str) -> dict[str, Any]:
    """Delete a quick check."""
    pool = await _get_pool()
    if not pool:
        return {"error": "DB unavailable"}

    await pool.execute("DELETE FROM quick_checks WHERE id = $1", check_id)
    return {"deleted": check_id}
