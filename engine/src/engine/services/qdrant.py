from __future__ import annotations

import logging
from typing import Any

from qdrant_client import QdrantClient
from qdrant_client.models import Distance, PointStruct, VectorParams

from engine.config import settings

logger = logging.getLogger("engine.qdrant")


class QdrantService:
    """Vector store operations backed by Qdrant."""

    def __init__(self) -> None:
        self._client: QdrantClient | None = None

    @property
    def client(self) -> QdrantClient:
        if self._client is None:
            self._client = QdrantClient(url=settings.qdrant_url)
            logger.info("Qdrant connected: %s", settings.qdrant_url)
        return self._client

    async def ensure_collection(self, dim: int = 768) -> None:
        """Create the default collection if it doesn't exist."""
        collections = self.client.get_collections().collections
        names = [c.name for c in collections]
        if settings.qdrant_collection not in names:
            self.client.create_collection(
                collection_name=settings.qdrant_collection,
                vectors_config=VectorParams(size=dim, distance=Distance.COSINE),
            )
            logger.info("Created collection %s (dim=%d)", settings.qdrant_collection, dim)

    async def upsert(self, points: list[dict[str, Any]]) -> int:
        """Upsert vectors. Each dict needs: id, vector, payload."""
        structs = [PointStruct(id=p["id"], vector=p["vector"], payload=p.get("payload", {})) for p in points]
        self.client.upsert(collection_name=settings.qdrant_collection, points=structs)
        return len(structs)

    async def search(
        self,
        vector: list[float],
        limit: int = 10,
        score_threshold: float | None = None,
        collection: str | None = None,
    ) -> list[dict[str, Any]]:
        """Nearest-neighbor search."""
        results = self.client.query_points(
            collection_name=collection or settings.qdrant_collection,
            query=vector,
            limit=limit,
            score_threshold=score_threshold,
        )
        return [{"id": hit.id, "score": hit.score, "payload": hit.payload} for hit in results.points]

    async def delete(self, ids: list[str | int], collection: str | None = None) -> None:
        from qdrant_client.models import PointIdsList

        self.client.delete(
            collection_name=collection or settings.qdrant_collection,
            points_selector=PointIdsList(points=ids),
        )

    async def collection_info(self, collection: str | None = None) -> dict[str, Any]:
        name = collection or settings.qdrant_collection
        info = self.client.get_collection(name)
        vectors = info.config.params.vectors if hasattr(info.config.params, "vectors") else None
        dim = None
        dist = None
        if vectors is not None:
            if hasattr(vectors, "size"):
                dim = vectors.size
                dist = vectors.distance.value if vectors.distance else None
            elif isinstance(vectors, dict):
                first = next(iter(vectors.values()), None)
                if first and hasattr(first, "size"):
                    dim = first.size
                    dist = first.distance.value if first.distance else None
        return {
            "name": name,
            "points_count": info.points_count,
            "status": info.status.value,
            "dimension": dim,
            "distance": dist,
        }


qdrant_service = QdrantService()
