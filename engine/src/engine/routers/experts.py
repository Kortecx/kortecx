"""Expert management API — CRUD, versioning, prompt files."""

from __future__ import annotations

import logging
from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.expert_manager import expert_manager

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


class RestoreVersionRequest(BaseModel):
    version: str


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
