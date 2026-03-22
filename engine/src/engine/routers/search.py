"""Global search API — search across workflows, experts, datasets."""
from __future__ import annotations

import logging
from typing import Any

import httpx
from fastapi import APIRouter

logger = logging.getLogger("engine.routers.search")
router = APIRouter()


@router.get("/")
async def global_search(q: str = "", limit: int = 20) -> dict[str, Any]:
    """Search across all platform entities."""
    if not q.strip():
        return {"results": [], "total": 0}

    results: list[dict[str, Any]] = []
    query = q.lower()

    # Search experts (from engine filesystem)
    try:
        from engine.services.expert_manager import expert_manager

        for expert in expert_manager.list_all():
            name = expert.get("name", "").lower()
            desc = expert.get("description", "").lower()
            role = expert.get("role", "").lower()
            if query in name or query in desc or query in role:
                results.append({
                    "type": "expert",
                    "id": expert.get("id"),
                    "name": expert.get("name"),
                    "description": expert.get("description", "")[:200],
                    "metadata": {"role": expert.get("role"), "source": expert.get("source", "engine")},
                })
    except Exception as e:
        logger.warning("Expert search failed: %s", e)

    # Search frontend entities
    try:
        async with httpx.AsyncClient(timeout=5) as client:
            resp = await client.get("http://localhost:3000/api/search", params={"q": q, "limit": limit})
            if resp.status_code == 200:
                data = resp.json()
                results.extend(data.get("results", []))
    except Exception:
        pass

    return {"results": results[:limit], "total": len(results), "query": q}
