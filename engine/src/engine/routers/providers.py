"""Provider sync API — fetch provider API keys from frontend DB."""
from __future__ import annotations

import logging
from typing import Any

import httpx
from fastapi import APIRouter

logger = logging.getLogger("engine.routers.providers")
router = APIRouter()


@router.get("/keys")
async def get_provider_keys() -> dict[str, Any]:
    """Fetch all active provider API keys from frontend."""
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            resp = await client.get(
                "http://localhost:3000/api/providers", params={"keys": "true"}
            )
            if resp.status_code == 200:
                return resp.json()
    except Exception as e:
        logger.error("Failed to fetch provider keys: %s", e)
    return {"providers": []}


@router.get("/status")
async def provider_status() -> dict[str, Any]:
    """Get provider health status."""
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            resp = await client.get("http://localhost:3000/api/providers")
            if resp.status_code == 200:
                return resp.json()
    except Exception as e:
        logger.error("Failed to fetch provider status: %s", e)
    return {"providers": []}
