"""Expert management API — CRUD, versioning, prompt files, execution."""

from __future__ import annotations

import asyncio
import logging
import uuid
from datetime import UTC, datetime
from typing import Any

import httpx
from fastapi import APIRouter, BackgroundTasks
from pydantic import BaseModel

from engine.services.expert_artifacts import expert_artifacts
from engine.services.expert_manager import expert_manager
from engine.services.expert_sync import expert_sync
from engine.services.local_inference import inference_router, model_pool

logger = logging.getLogger("engine.routers.experts")

router = APIRouter()


# ── Request models ───────────────────────────────────────────────────────────


class CreateExpertRequest(BaseModel):
    name: str
    role: str
    description: str = ""
    systemPrompt: str = ""
    userPrompt: str = ""
    modelSource: str = "local"
    localModelConfig: dict[str, str] | None = None
    temperature: float = 0.7
    maxTokens: int = 4096
    tags: list[str] = []
    capabilities: list[str] = []
    isPublic: bool = False
    category: str = "custom"


class UpdateFileRequest(BaseModel):
    filename: str
    content: str


class SaveRunArtifactRequest(BaseModel):
    expertName: str
    response: str
    prompt: str = ""
    systemPrompt: str = ""
    model: str = ""
    engine: str = ""
    tokensUsed: int = 0
    durationMs: float = 0
    tags: list[str] = []
    metadata: dict[str, Any] | None = None


class ExecuteExpertRequest(BaseModel):
    expertName: str
    model: str = "llama3.2:3b"
    engine: str = "ollama"
    temperature: float = 0.7
    maxTokens: int = 4096
    systemPrompt: str = ""
    userPrompt: str = ""
    tags: list[str] = []
    metadata: dict[str, Any] | None = None
    callbackUrl: str | None = None


class RestoreVersionRequest(BaseModel):
    version: str


# ── In-memory run tracking ───────────────────────────────────────────────────

_expert_runs: dict[str, dict[str, Any]] = {}


async def _send_callback(url: str, payload: dict[str, Any], run_id: str, retries: int = 3) -> bool:
    """Send callback to frontend with retry logic (exponential backoff)."""
    for attempt in range(retries):
        try:
            async with httpx.AsyncClient(timeout=10) as client:
                resp = await client.post(url, json={**payload, "metadata": payload.get("metadata", {})})
                if resp.status_code < 300:
                    logger.info("Callback succeeded for %s (attempt %d)", run_id, attempt + 1)
                    return True
                logger.warning(
                    "Callback returned %d for %s (attempt %d)",
                    resp.status_code,
                    run_id,
                    attempt + 1,
                )
        except Exception as e:
            logger.warning("Callback failed for %s (attempt %d): %s", run_id, attempt + 1, e)
        if attempt < retries - 1:
            await asyncio.sleep(2**attempt)  # 1s, 2s backoff
    logger.error("Callback permanently failed for %s after %d attempts", run_id, retries)
    return False


def _cleanup_old_runs() -> int:
    """Remove completed/failed runs older than 1 hour from in-memory tracking."""
    one_hour_ago = datetime.now(UTC).timestamp() - 3600
    to_remove = []
    for rid, data in _expert_runs.items():
        if data.get("status") in ("completed", "failed"):
            completed_at = data.get("completedAt", "")
            if completed_at:
                try:
                    completed_ts = datetime.fromisoformat(completed_at.replace("Z", "+00:00")).timestamp()
                    if completed_ts < one_hour_ago:
                        to_remove.append(rid)
                except (ValueError, TypeError):
                    to_remove.append(rid)
    for rid in to_remove:
        del _expert_runs[rid]
    if to_remove:
        logger.info("Cleaned up %d old expert runs from memory", len(to_remove))
    return len(to_remove)


async def _run_expert_background(run_id: str, expert_id: str, req: ExecuteExpertRequest) -> None:
    """Background task: run inference, save artifacts, callback to frontend."""
    _expert_runs[run_id]["status"] = "running"
    try:
        # Acquire model slot and run inference
        await model_pool.acquire(req.model)
        try:
            messages = []
            if req.systemPrompt:
                messages.append({"role": "system", "content": req.systemPrompt})
            messages.append({"role": "user", "content": req.userPrompt})

            result = await inference_router.chat(
                engine=req.engine,
                model=req.model,
                messages=messages,
                temperature=req.temperature,
                max_tokens=req.maxTokens,
            )
        finally:
            model_pool.release(req.model)

        response_text = result.text
        tokens_used = result.tokens_used
        duration_ms = result.duration_ms

        # Persist artifacts to disk
        artifact_result = expert_artifacts.save_response(
            expert_id=expert_id,
            expert_name=req.expertName,
            response=response_text,
            prompt=req.userPrompt,
            system_prompt=req.systemPrompt,
            model=req.model,
            engine=req.engine,
            tokens_used=tokens_used,
            duration_ms=duration_ms,
            tags=req.tags,
            metadata=req.metadata,
        )

        _expert_runs[run_id].update(
            {
                "status": "completed",
                "responseText": response_text,
                "tokensUsed": tokens_used,
                "durationMs": duration_ms,
                "completedAt": datetime.now(UTC).isoformat(),
                "artifacts": artifact_result,
            }
        )

        # Callback to frontend with results (retries on failure)
        if req.callbackUrl:
            await _send_callback(
                req.callbackUrl,
                {
                    "runId": run_id,
                    "expertId": expert_id,
                    "expertName": req.expertName,
                    "status": "completed",
                    "responseText": response_text,
                    "tokensUsed": tokens_used,
                    "durationMs": duration_ms,
                    "model": req.model,
                    "engine": req.engine,
                    "artifacts": artifact_result,
                    "metadata": req.metadata or {},
                },
                run_id,
            )

    except Exception as e:
        logger.exception("Expert run failed for %s (%s)", expert_id, run_id)
        _expert_runs[run_id].update(
            {
                "status": "failed",
                "errorMessage": str(e),
                "completedAt": datetime.now(UTC).isoformat(),
            }
        )

        # Callback with failure (retries on failure)
        if req.callbackUrl:
            await _send_callback(
                req.callbackUrl,
                {
                    "runId": run_id,
                    "expertId": expert_id,
                    "expertName": req.expertName,
                    "status": "failed",
                    "errorMessage": str(e),
                    "metadata": req.metadata or {},
                },
                run_id,
            )


# ── Helpers ──────────────────────────────────────────────────────────────────


def _clean(expert: dict[str, Any]) -> dict[str, Any]:
    """Strip internal fields (prefixed with _) from expert data."""
    return {k: v for k, v in expert.items() if not k.startswith("_")}


# ── Endpoints ────────────────────────────────────────────────────────────────


@router.get("/list")
async def list_experts() -> dict[str, Any]:
    """List all experts from marketplace and local."""
    experts = expert_manager.load_all()
    marketplace = [_clean(e) for e in experts if e.get("_source") == "marketplace"]
    local = [_clean(e) for e in experts if e.get("_source") == "local"]
    return {
        "marketplace": marketplace,
        "local": local,
        "total": len(experts),
    }


@router.post("/engine/sync")
async def sync_experts_to_db() -> dict[str, Any]:
    """Trigger a full sync of all engine filesystem experts to PostgreSQL."""
    if not expert_sync.available:
        try:
            await expert_sync.connect()
        except Exception:
            logger.exception("Could not connect ExpertSyncService for bulk sync")
            return {"error": "Database connection unavailable", "synced": 0}
    result = await expert_manager.sync_all_to_db()
    return result


@router.get("/{expert_id}")
async def get_expert(expert_id: str) -> dict[str, Any]:
    """Get a single expert with its prompts."""
    expert = expert_manager.get(expert_id)
    if not expert:
        return {"error": "Expert not found"}

    system = expert_manager.get_prompt(expert_id, "system")
    user = expert_manager.get_prompt(expert_id, "user")
    files = expert_manager.list_files(expert_id)

    result = _clean(expert)
    result["systemPrompt"] = system
    result["userPrompt"] = user
    result["files"] = files
    return result


@router.post("/create")
async def create_expert(req: CreateExpertRequest) -> dict[str, Any]:
    """Create a new local expert."""
    expert = expert_manager.create_local(
        name=req.name,
        role=req.role,
        config={
            "description": req.description,
            "systemPrompt": req.systemPrompt,
            "userPrompt": req.userPrompt,
            "modelSource": req.modelSource,
            "localModelConfig": req.localModelConfig or {"engine": "ollama", "modelName": "llama3.2:3b"},
            "temperature": req.temperature,
            "maxTokens": req.maxTokens,
            "tags": req.tags,
            "capabilities": req.capabilities,
            "isPublic": req.isPublic,
            "category": req.category,
        },
    )
    return {"expert": _clean(expert)}


@router.post("/{expert_id}/update")
async def update_expert_file(expert_id: str, req: UpdateFileRequest) -> dict[str, Any]:
    """Update a single file with per-file versioning."""
    try:
        result = expert_manager.update_file(expert_id, req.filename, req.content)
    except ValueError as e:
        return {"error": str(e)}
    return result


@router.get("/{expert_id}/versions/{filename}")
async def list_versions(expert_id: str, filename: str) -> dict[str, Any]:
    """List all versions of a specific file."""
    versions = expert_manager.get_versions(expert_id, filename)
    return {"versions": versions, "total": len(versions)}


@router.post("/{expert_id}/restore")
async def restore_version(
    expert_id: str,
    body: RestoreVersionRequest,
) -> dict[str, Any]:
    """Restore a file from a version."""
    if not body.version:
        return {"error": "version filename required"}
    try:
        result = expert_manager.restore_version(expert_id, body.version)
    except ValueError as e:
        return {"error": str(e)}
    return result


@router.get("/{expert_id}/files")
async def list_expert_files(expert_id: str) -> dict[str, Any]:
    """List all files in an expert's directory."""
    files = expert_manager.list_files(expert_id)
    return {"files": files, "total": len(files)}


@router.patch("/{expert_id}/versions/config")
async def update_version_config(expert_id: str, body: dict[str, Any]) -> dict[str, Any]:
    """Update the maxVersions setting for an expert."""
    expert = expert_manager.get(expert_id)
    if not expert:
        return {"error": f"Expert {expert_id} not found"}

    max_versions = body.get("maxVersions", 50)
    if not isinstance(max_versions, int) or max_versions < 1:
        max_versions = 50

    import json
    from pathlib import Path

    ej_path = Path(expert["_dir"]) / "expert.json"
    if ej_path.exists():
        data = json.loads(ej_path.read_text(encoding="utf-8"))
        data["maxVersions"] = max_versions
        ej_path.write_text(json.dumps(data, indent=2), encoding="utf-8")

    return {"ok": True, "maxVersions": max_versions}


@router.get("/{expert_id}/prompt/{prompt_type}")
async def get_prompt(expert_id: str, prompt_type: str) -> dict[str, Any]:
    """Get a specific prompt file (system or user)."""
    content = expert_manager.get_prompt(expert_id, prompt_type)
    return {"content": content, "type": prompt_type}


@router.delete("/{expert_id}")
async def delete_expert(expert_id: str) -> dict[str, Any]:
    """Delete a local expert."""
    try:
        deleted = expert_manager.delete_expert(expert_id)
    except ValueError as e:
        return {"error": str(e)}
    return {"deleted": deleted, "id": expert_id}


# ── Execution Endpoints ─────────────────────────────────────────────────────


@router.post("/{expert_id}/execute")
async def execute_expert(expert_id: str, req: ExecuteExpertRequest, bg: BackgroundTasks) -> dict[str, Any]:
    """Start expert execution in background — returns immediately with a runId."""
    _cleanup_old_runs()
    run_id = f"er-{uuid.uuid4().hex[:12]}"
    _expert_runs[run_id] = {
        "runId": run_id,
        "expertId": expert_id,
        "expertName": req.expertName,
        "status": "started",
        "model": req.model,
        "engine": req.engine,
        "startedAt": datetime.now(UTC).isoformat(),
    }
    bg.add_task(_run_expert_background, run_id, expert_id, req)
    return {"runId": run_id, "status": "started"}


@router.get("/{expert_id}/execute/{run_id}")
async def get_expert_run_status(expert_id: str, run_id: str) -> dict[str, Any]:
    """Get the status of a running or completed expert execution."""
    run = _expert_runs.get(run_id)
    if not run:
        return {"error": "Run not found", "runId": run_id}
    return run


# ── Artifact Endpoints ──────────────────────────────────────────────────────


@router.get("/artifacts/all")
async def list_all_artifacts(
    expert_name: str | None = None,
    date: str | None = None,
    file_type: str | None = None,
) -> dict[str, Any]:
    """List all expert artifacts across all experts and dates."""
    artifacts = expert_artifacts.list_artifacts(expert_name=expert_name, date=date)
    if file_type:
        artifacts = [a for a in artifacts if a.get("fileType") == file_type]
    return {"artifacts": artifacts, "total": len(artifacts)}


@router.post("/{expert_id}/run-artifact")
async def save_run_artifact(expert_id: str, req: SaveRunArtifactRequest) -> dict[str, Any]:
    """Save full expert run output locally with date-based organization."""
    try:
        result = expert_artifacts.save_response(
            expert_id=expert_id,
            expert_name=req.expertName,
            response=req.response,
            prompt=req.prompt,
            system_prompt=req.systemPrompt,
            model=req.model,
            engine=req.engine,
            tokens_used=req.tokensUsed,
            duration_ms=req.durationMs,
            tags=req.tags,
            metadata=req.metadata,
        )
        return result
    except Exception as e:
        logger.exception("Failed to save expert run artifact for %s", expert_id)
        return {"error": str(e)}


@router.get("/{expert_id}/artifacts")
async def list_expert_artifacts(expert_id: str, date: str | None = None) -> dict[str, Any]:
    """List artifacts for a specific expert."""
    expert = expert_manager.get(expert_id)
    expert_name = expert.get("name", expert_id) if expert else expert_id
    artifacts = expert_artifacts.list_artifacts(expert_name=expert_name, date=date)
    return {"artifacts": artifacts, "total": len(artifacts)}
