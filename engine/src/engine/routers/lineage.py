"""Data lineage API — track downstream impact of data changes."""

from __future__ import annotations

import logging
from typing import Any

import httpx
from fastapi import APIRouter

logger = logging.getLogger("engine.routers.lineage")
router = APIRouter()


@router.get("/graph")
async def get_lineage_graph(source_type: str = "", source_id: str = "") -> dict[str, Any]:
    """Get lineage graph for a source entity."""
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            resp = await client.get(
                "http://localhost:3000/api/lineage",
                params={"sourceType": source_type, "sourceId": source_id},
            )
            if resp.status_code == 200:
                return resp.json()
    except Exception as e:
        logger.error("Failed to fetch lineage: %s", e)
    return {"nodes": [], "edges": []}


@router.get("/impact")
async def impact_analysis(entity_type: str, entity_id: str) -> dict[str, Any]:
    """Analyze downstream impact if an entity is modified or deleted."""
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            resp = await client.get(
                "http://localhost:3000/api/lineage",
                params={"targetType": entity_type, "targetId": entity_id, "direction": "downstream"},
            )
            if resp.status_code == 200:
                data = resp.json()
                downstream = data.get("lineage", [])
                return {
                    "entityType": entity_type,
                    "entityId": entity_id,
                    "downstreamCount": len(downstream),
                    "downstream": downstream,
                    "safe_to_modify": len(downstream) == 0,
                }
    except Exception as e:
        logger.error("Failed to analyze impact: %s", e)
    return {"entityType": entity_type, "entityId": entity_id, "downstreamCount": 0, "downstream": [], "safe_to_modify": True}
