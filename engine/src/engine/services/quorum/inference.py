"""Quorum inference client — wraps existing local inference backends with quorum-specific capabilities."""

from __future__ import annotations

import asyncio
import logging
import os
from typing import Any

from engine.services.local_inference import (
    LlamaCppService,
    OllamaService,
    model_pool,
)
from engine.services.quorum.errors import InferenceError
from engine.services.quorum.types import CompletionResponse

logger = logging.getLogger("engine.quorum.inference")


class QuorumInferenceClient:
    """Distributed inference client with load tracking and backend routing.

    Wraps the existing OllamaService and LlamaCppService from local_inference
    with quorum-specific concurrency management via the shared ModelPool.
    """

    def __init__(self, ollama_url: str, llamacpp_url: str) -> None:
        self._ollama = OllamaService(ollama_url)
        self._llamacpp = LlamaCppService(llamacpp_url)
        self._pool = model_pool  # reuse existing model pool singleton
        self._semaphore: asyncio.Semaphore | None = None

    def _get_backend(self, backend: str) -> OllamaService | LlamaCppService:
        """Resolve a backend name to the corresponding service instance."""
        if backend == "ollama":
            return self._ollama
        if backend == "llamacpp":
            return self._llamacpp
        raise InferenceError(f"Unknown inference backend: {backend}")

    async def complete(
        self,
        *,
        backend: str,
        model: str,
        prompt: str,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 2048,
    ) -> CompletionResponse:
        """Execute a completion request against the specified backend.

        Acquires a slot in the model pool before sending the request,
        ensuring we track concurrent load per model.
        """
        svc = self._get_backend(backend)
        await self._pool.acquire(model)
        try:
            result = await svc.generate(
                model,
                prompt,
                system=system,
                temperature=temperature,
                max_tokens=max_tokens,
            )
            return CompletionResponse(
                text=result.text,
                tokens_used=result.tokens_used,
                model=result.model,
                duration_ms=int(result.duration_ms),
            )
        except InferenceError:
            raise
        except Exception as e:
            raise InferenceError(f"Inference failed on {backend}/{model}: {e}") from e
        finally:
            await self._pool.release(model)

    async def health(self, backend: str) -> bool:
        """Check if the specified backend is healthy."""
        try:
            return await self._get_backend(backend).health()
        except InferenceError:
            return False

    async def list_models(self, backend: str) -> list[dict[str, Any]]:
        """List available models on the specified backend."""
        svc = self._get_backend(backend)
        return await svc.list_models()

    async def pull_model(self, model: str, backend: str) -> None:
        """Pull a model from the registry. Currently only supported for Ollama."""
        if backend == "ollama":
            await self._ollama.pull_model(model)
        else:
            raise InferenceError("Model pull is only supported for the Ollama backend")

    def get_parallel_capacity(self) -> int:
        """Return the Ollama parallel request capacity from OLLAMA_NUM_PARALLEL."""
        return int(os.environ.get("OLLAMA_NUM_PARALLEL", "1"))

    @property
    def active_requests(self) -> dict[str, int]:
        """Return the current active request counts per model."""
        return self._pool.active_models

    @property
    def total_active(self) -> int:
        """Return the total number of active inference requests across all models."""
        return self._pool.total_active
