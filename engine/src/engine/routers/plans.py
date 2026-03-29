"""Plans API — generate, save, freeze/unfreeze, list versions, resolve execution plan."""

from __future__ import annotations

import logging
import re
from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services import plan_generator, plan_store

logger = logging.getLogger("engine.routers.plans")

router = APIRouter()


def _slugify(name: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-") or "untitled"


# ── Request Models ──────────────────────────────────────────────────────────


class GenerateRequest(BaseModel):
    workflowId: str | None = None
    workflowSlug: str | None = None
    prompt: str | None = None
    useGraph: bool = True
    model: str = "llama3.1:8b"
    engine: str = "ollama"


class SaveRequest(BaseModel):
    workflowSlug: str
    dag: dict[str, Any]
    markdown: str | None = None
    maxVersions: int = 3


class FreezeRequest(BaseModel):
    workflowSlug: str
    action: str  # 'freeze' | 'unfreeze' | 'refreeze'
    version: int | None = None


# ── Endpoints ───────────────────────────────────────────────────────────────


@router.post("/generate")
async def generate_plan(req: GenerateRequest) -> dict[str, Any]:
    """Generate a plan DAG from the PRISM graph, optionally guided by a prompt."""
    dag = await plan_generator.generate_plan_from_graph(
        workflow_id=req.workflowId,
        prompt=req.prompt if req.prompt else None,
        model=req.model,
        engine=req.engine,
    )
    return {"dag": dag, "generatedBy": dag.get("generatedBy", "prism_graph")}


@router.post("/save")
async def save_plan(req: SaveRequest) -> dict[str, Any]:
    """Save a plan to the LIVE directory with version rotation."""
    result = plan_store.save_live_plan(
        slug=req.workflowSlug,
        dag=req.dag,
        markdown=req.markdown,
        max_versions=req.maxVersions,
    )
    return {"saved": True, **result}


@router.post("/freeze")
async def freeze_plan(req: FreezeRequest) -> dict[str, Any]:
    """Freeze, unfreeze, or refreeze a plan."""
    if req.action == "freeze":
        result = plan_store.freeze_plan(req.workflowSlug)
        if not result:
            return {"error": "No LIVE plan to freeze", "frozen": False}
        return {"frozen": True, **result}

    elif req.action == "unfreeze":
        removed = plan_store.unfreeze_plan(req.workflowSlug)
        return {"frozen": False, "removed": removed}

    elif req.action == "refreeze":
        result = plan_store.refreeze_plan(req.workflowSlug, version=req.version)
        if not result:
            return {"error": "No plan found to refreeze", "frozen": False}
        return {"frozen": True, **result}

    return {"error": f"Unknown action: {req.action}"}


@router.get("/live/{slug}")
async def get_live_plan(slug: str) -> dict[str, Any]:
    """Get the latest LIVE plan for a workflow."""
    plan = plan_store.get_live_plan(slug)
    return {"plan": plan} if plan else {"plan": None}


@router.get("/frozen/{slug}")
async def get_frozen_plan(slug: str) -> dict[str, Any]:
    """Get the FREEZE plan for a workflow."""
    plan = plan_store.get_frozen_plan(slug)
    return {"plan": plan} if plan else {"plan": None}


@router.get("/versions/{slug}")
async def list_versions(slug: str) -> dict[str, Any]:
    """List all LIVE plan versions for a workflow."""
    versions = plan_store.list_live_versions(slug)
    return {"versions": versions, "total": len(versions)}


@router.get("/version/{slug}/{version}")
async def get_version(slug: str, version: int) -> dict[str, Any]:
    """Get a specific LIVE plan version."""
    plan = plan_store.get_live_plan_version(slug, version)
    return {"plan": plan} if plan else {"plan": None}


@router.get("/resolve/{slug}")
async def resolve_plan(slug: str, frozen: bool = False) -> dict[str, Any]:
    """Get the execution plan — returns FREEZE if frozen, else LIVE."""
    plan = plan_store.get_execution_plan(slug, is_frozen=frozen)
    return {"plan": plan, "type": "frozen" if frozen and plan else "live"}
