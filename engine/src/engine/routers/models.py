from __future__ import annotations

from typing import Any

from fastapi import APIRouter, Query

from engine.services.hf import hf_service

router = APIRouter()


@router.get("/search")
async def search_models(
    query: str = "",
    pipeline_tag: str | None = None,
    library: str | None = None,
    sort: str = "downloads",
    limit: int = Query(20, ge=1, le=100),
) -> dict[str, Any]:
    """Search HuggingFace Hub models."""
    models = hf_service.search_models(
        query=query,
        pipeline_tag=pipeline_tag,
        library=library,
        sort=sort,
        limit=limit,
    )
    return {"models": models, "count": len(models)}


@router.get("/{model_id:path}")
async def get_model_info(model_id: str) -> dict[str, Any]:
    """Get detailed info for a specific model."""
    return hf_service.get_model_info(model_id)
