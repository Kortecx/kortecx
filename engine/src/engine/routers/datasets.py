"""HuggingFace dataset operations — search, download, preview, info."""

from __future__ import annotations

import asyncio
import logging
from typing import Any

from fastapi import APIRouter, Header, Query
from pydantic import BaseModel

from engine.services.hf import hf_service

logger = logging.getLogger("engine.datasets")

router = APIRouter()


class DatasetDownloadRequest(BaseModel):
    dataset_id: str
    config: str | None = None
    split: str | None = None


class DatasetPreviewRequest(BaseModel):
    dataset_id: str
    config: str | None = None
    split: str = "train"
    rows: int = 20


@router.get("/status")
async def hf_status() -> dict[str, Any]:
    """Check if HuggingFace is configured with an API token."""
    return {
        "configured": hf_service.has_token,
        "tokenSet": bool(hf_service._token),
    }


@router.get("/search")
async def search_datasets(
    query: str = "",
    sort: str = "downloads",
    limit: int = Query(20, ge=1, le=100),
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Search HuggingFace Hub datasets."""
    if x_hf_token:
        hf_service.set_token(x_hf_token)
    datasets = hf_service.search_datasets(query=query, sort=sort, limit=limit)
    return {"datasets": datasets, "count": len(datasets)}


@router.post("/download")
async def download_dataset(
    body: DatasetDownloadRequest,
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Download dataset to HF cache and return metadata."""
    from fastapi.responses import JSONResponse

    if x_hf_token:
        hf_service.set_token(x_hf_token)
    elif not hf_service.has_token:
        return JSONResponse(
            status_code=401,
            content={"error": "HuggingFace API token not configured. Set it in Providers → Hugging Face."},
        )
    try:
        result = await asyncio.to_thread(hf_service.download_dataset, body.dataset_id, body.config, body.split)
        return result
    except Exception as exc:
        logger.exception("Dataset download failed: %s", body.dataset_id)
        return JSONResponse(
            status_code=500,
            content={"error": str(exc), "dataset_id": body.dataset_id},
        )


@router.post("/preview")
async def preview_dataset(
    body: DatasetPreviewRequest,
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Preview rows of a downloaded dataset."""
    if x_hf_token:
        hf_service.set_token(x_hf_token)
    result = await asyncio.to_thread(hf_service.get_dataset_preview, body.dataset_id, body.config, body.split, body.rows)
    return result


@router.get("/{dataset_id:path}")
async def get_dataset_info(
    dataset_id: str,
    x_hf_token: str | None = Header(None),
) -> dict[str, Any]:
    """Get detailed info for a specific dataset."""
    if x_hf_token:
        hf_service.set_token(x_hf_token)
    return hf_service.get_dataset_info(dataset_id)
