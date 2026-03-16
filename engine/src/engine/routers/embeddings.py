from __future__ import annotations

from typing import Any

from fastapi import APIRouter
from pydantic import BaseModel

from engine.services.hf import hf_service
from engine.services.qdrant import qdrant_service

router = APIRouter()


class EmbedRequest(BaseModel):
    texts: list[str]
    model_id: str = "sentence-transformers/all-MiniLM-L6-v2"


class UpsertRequest(BaseModel):
    texts: list[str]
    model_id: str = "sentence-transformers/all-MiniLM-L6-v2"
    payloads: list[dict[str, Any]] | None = None
    collection: str | None = None


class SearchRequest(BaseModel):
    query: str
    model_id: str = "sentence-transformers/all-MiniLM-L6-v2"
    limit: int = 10
    score_threshold: float | None = None
    collection: str | None = None


@router.post("/embed")
async def embed_texts(req: EmbedRequest) -> dict[str, Any]:
    """Generate embeddings for texts."""
    vectors = hf_service.text_embedding(req.model_id, req.texts)
    return {"vectors": vectors, "model": req.model_id, "count": len(vectors)}


@router.post("/upsert")
async def upsert_embeddings(req: UpsertRequest) -> dict[str, Any]:
    """Embed texts and store in Qdrant."""
    vectors = hf_service.text_embedding(req.model_id, req.texts)
    payloads = req.payloads or [{"text": t} for t in req.texts]

    points = [{"id": i, "vector": v, "payload": p} for i, (v, p) in enumerate(zip(vectors, payloads))]
    count = await qdrant_service.upsert(points)
    return {"upserted": count, "model": req.model_id}


@router.post("/search")
async def search_similar(req: SearchRequest) -> dict[str, Any]:
    """Semantic search — embed query and search Qdrant."""
    vectors = hf_service.text_embedding(req.model_id, req.query)
    query_vector = vectors[0] if vectors else []

    results = await qdrant_service.search(
        vector=query_vector,
        limit=req.limit,
        score_threshold=req.score_threshold,
        collection=req.collection,
    )
    return {"results": results, "count": len(results)}


@router.get("/collections")
async def collection_info(collection: str | None = None) -> dict[str, Any]:
    """Get info about a Qdrant collection."""
    return await qdrant_service.collection_info(collection)
