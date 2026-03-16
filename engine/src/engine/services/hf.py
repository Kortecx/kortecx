from __future__ import annotations

import logging
from typing import Any

from huggingface_hub import HfApi, InferenceClient

from engine.config import settings

logger = logging.getLogger("engine.hf")


class HuggingFaceService:
    """HuggingFace Hub + Inference integration."""

    def __init__(self) -> None:
        self._api: HfApi | None = None
        self._inference: InferenceClient | None = None

    @property
    def api(self) -> HfApi:
        if self._api is None:
            self._api = HfApi(token=settings.hf_token or None)
        return self._api

    @property
    def inference(self) -> InferenceClient:
        if self._inference is None:
            self._inference = InferenceClient(token=settings.hf_token or None)
        return self._inference

    def search_models(
        self,
        query: str = "",
        pipeline_tag: str | None = None,
        library: str | None = None,
        sort: str = "downloads",
        limit: int = 20,
    ) -> list[dict[str, Any]]:
        models = self.api.list_models(
            search=query or None,
            pipeline_tag=pipeline_tag,
            library=library,
            sort=sort,
            limit=limit,
        )
        return [
            {
                "id": m.id,
                "pipeline_tag": m.pipeline_tag,
                "downloads": m.downloads,
                "likes": m.likes,
                "tags": m.tags,
            }
            for m in models
        ]

    def search_datasets(
        self,
        query: str = "",
        sort: str = "downloads",
        limit: int = 20,
    ) -> list[dict[str, Any]]:
        datasets = self.api.list_datasets(search=query or None, sort=sort, limit=limit)
        return [
            {
                "id": d.id,
                "downloads": d.downloads,
                "likes": d.likes,
                "tags": d.tags,
            }
            for d in datasets
        ]

    def get_model_info(self, model_id: str) -> dict[str, Any]:
        info = self.api.model_info(model_id)
        return {
            "id": info.id,
            "pipeline_tag": info.pipeline_tag,
            "downloads": info.downloads,
            "likes": info.likes,
            "tags": info.tags,
            "library_name": info.library_name,
            "created_at": str(info.created_at) if info.created_at else None,
            "last_modified": str(info.last_modified) if info.last_modified else None,
        }

    async def run_inference(self, model_id: str, inputs: Any, parameters: dict[str, Any] | None = None) -> Any:
        """Run inference on a HuggingFace hosted model."""
        return self.inference.post(
            model=model_id,
            json={"inputs": inputs, "parameters": parameters or {}},
        )

    def text_generation(self, model_id: str, prompt: str, **kwargs: Any) -> str:
        result = self.inference.text_generation(prompt, model=model_id, **kwargs)
        return result

    def text_embedding(self, model_id: str, text: str | list[str]) -> list[list[float]]:
        """Get embeddings from a HuggingFace model."""
        result = self.inference.feature_extraction(text, model=model_id)
        if isinstance(result[0], float):
            return [result]
        return result


hf_service = HuggingFaceService()
