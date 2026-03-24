"""Local inference backends — Ollama and llama.cpp server wrappers."""

from __future__ import annotations

import asyncio
import json
import logging
from abc import ABC, abstractmethod
from collections.abc import AsyncIterator
from dataclasses import dataclass
from typing import Any

import httpx

from engine.config import settings

logger = logging.getLogger("engine.local_inference")

# ── Retry helper ──────────────────────────────────────────────────────────────

MAX_RETRIES = 2
RETRY_DELAY = 1.0


async def _with_retry(coro_factory, retries: int = MAX_RETRIES) -> Any:
    """Retry an async operation with exponential backoff."""
    last_exc = None
    for attempt in range(retries + 1):
        try:
            return await coro_factory()
        except (httpx.ConnectError, httpx.ReadTimeout, httpx.ConnectTimeout, RuntimeError) as exc:
            last_exc = exc
            if attempt < retries:
                delay = RETRY_DELAY * (2**attempt)
                logger.warning("Inference retry %d/%d after %.1fs: %s", attempt + 1, retries, delay, exc)
                await asyncio.sleep(delay)
    raise last_exc  # type: ignore[misc]


# ── Model pool ────────────────────────────────────────────────────────────────


class ModelPool:
    """Track which models are loaded and manage capacity for concurrent agents."""

    def __init__(self) -> None:
        self._active_requests: dict[str, int] = {}  # model -> active request count
        self._lock = asyncio.Lock()

    async def acquire(self, model: str) -> None:
        """Register an active inference request for a model."""
        async with self._lock:
            self._active_requests[model] = self._active_requests.get(model, 0) + 1

    async def release(self, model: str) -> None:
        """Mark an inference request as complete."""
        async with self._lock:
            count = self._active_requests.get(model, 0)
            if count <= 1:
                self._active_requests.pop(model, None)
            else:
                self._active_requests[model] = count - 1

    @property
    def active_models(self) -> dict[str, int]:
        return dict(self._active_requests)

    @property
    def total_active(self) -> int:
        return sum(self._active_requests.values())


model_pool = ModelPool()


# ── Data types ────────────────────────────────────────────────────────────────


@dataclass
class GenerateResult:
    text: str
    tokens_used: int
    model: str
    duration_ms: float


class LocalInferenceBackend(ABC):
    """Common interface for local LLM servers."""

    @abstractmethod
    async def generate(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult: ...

    @abstractmethod
    async def generate_stream(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> AsyncIterator[str]:
        """Yield token chunks as they arrive from the LLM."""
        ...
        # Make abstract generators work
        if False:
            yield ""  # pragma: no cover

    @abstractmethod
    async def chat(
        self,
        model: str,
        messages: list[dict[str, str]],
        *,
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult: ...

    @abstractmethod
    async def list_models(self) -> list[dict[str, Any]]: ...

    @abstractmethod
    async def health(self) -> bool: ...


# ── Ollama backend ────────────────────────────────────────────────────────────


class OllamaService(LocalInferenceBackend):
    """Wraps the Ollama REST API (http://localhost:11434)."""

    def __init__(self, base_url: str | None = None) -> None:
        self.base_url = (base_url or settings.ollama_url).rstrip("/")
        self._client: httpx.AsyncClient | None = None

    @property
    def client(self) -> httpx.AsyncClient:
        if self._client is None or self._client.is_closed:
            self._client = httpx.AsyncClient(base_url=self.base_url, timeout=300)
        return self._client

    async def generate(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult:
        payload: dict[str, Any] = {
            "model": model,
            "prompt": prompt,
            "stream": False,
            "options": {
                "temperature": float(temperature),
                "num_predict": max_tokens,
            },
        }
        if system:
            payload["system"] = system

        async def _do() -> httpx.Response:
            resp = await self.client.post("/api/generate", json=payload)
            resp.raise_for_status()
            return resp

        resp = await _with_retry(_do)
        data = resp.json()

        return GenerateResult(
            text=data.get("response", ""),
            tokens_used=data.get("eval_count", 0) + data.get("prompt_eval_count", 0),
            model=model,
            duration_ms=data.get("total_duration", 0) / 1_000_000,  # ns -> ms
        )

    async def generate_stream(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> AsyncIterator[str]:
        payload: dict[str, Any] = {
            "model": model,
            "prompt": prompt,
            "stream": True,
            "options": {"temperature": float(temperature), "num_predict": max_tokens},
        }
        if system:
            payload["system"] = system
        async with self.client.stream("POST", "/api/generate", json=payload) as resp:
            resp.raise_for_status()
            async for line in resp.aiter_lines():
                if not line.strip():
                    continue
                try:
                    chunk = json.loads(line)
                    token = chunk.get("response", "")
                    if token:
                        yield token
                    if chunk.get("done", False):
                        return
                except json.JSONDecodeError:
                    continue

    async def chat(
        self,
        model: str,
        messages: list[dict[str, str]],
        *,
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult:
        payload = {
            "model": model,
            "messages": messages,
            "stream": False,
            "options": {
                "temperature": float(temperature),
                "num_predict": max_tokens,
            },
        }

        async def _do() -> httpx.Response:
            resp = await self.client.post("/api/chat", json=payload)
            if resp.status_code >= 400:
                body = resp.text[:500]
                logger.error(
                    "Ollama error %d for model %s: %s",
                    resp.status_code, model, body,
                )
                raise RuntimeError(
                    f"Ollama returned {resp.status_code} for model '{model}': {body}"
                )
            return resp

        resp = await _with_retry(_do)
        data = resp.json()

        msg = data.get("message", {})
        return GenerateResult(
            text=msg.get("content", ""),
            tokens_used=data.get("eval_count", 0) + data.get("prompt_eval_count", 0),
            model=model,
            duration_ms=data.get("total_duration", 0) / 1_000_000,
        )

    async def list_models(self) -> list[dict[str, Any]]:
        resp = await self.client.get("/api/tags")
        resp.raise_for_status()
        data = resp.json()
        return [
            {
                "name": m["name"],
                "size": m.get("size", 0),
                "modified_at": m.get("modified_at", ""),
                "digest": m.get("digest", ""),
            }
            for m in data.get("models", [])
        ]

    async def pull_model(self, model: str) -> None:
        """Pull a model from the Ollama registry."""
        # Use a longer timeout for pulls — models can be large
        pull_client = httpx.AsyncClient(base_url=self.base_url, timeout=600)
        try:
            resp = await pull_client.post("/api/pull", json={"name": model, "stream": False})
            resp.raise_for_status()
        finally:
            await pull_client.aclose()

    async def get_model_info(self, model: str) -> dict[str, Any]:
        """Get detailed info about a specific model."""
        resp = await self.client.post("/api/show", json={"name": model})
        resp.raise_for_status()
        data = resp.json()
        return {
            "name": model,
            "family": data.get("details", {}).get("family", ""),
            "parameter_size": data.get("details", {}).get("parameter_size", ""),
            "quantization": data.get("details", {}).get("quantization_level", ""),
            "format": data.get("details", {}).get("format", ""),
            "template": data.get("template", ""),
            "context_length": data.get("model_info", {}).get("general.context_length", 0),
        }

    async def copy_model(self, source: str, destination: str) -> None:
        """Create a copy of a model with a new name (useful for creating expert-specific variants)."""
        resp = await self.client.post("/api/copy", json={"source": source, "destination": destination})
        resp.raise_for_status()

    async def delete_model(self, model: str) -> None:
        """Delete a model from Ollama."""
        resp = await self.client.request("DELETE", "/api/delete", json={"name": model})
        resp.raise_for_status()

    async def health(self) -> bool:
        try:
            resp = await self.client.get("/")
            return resp.status_code == 200
        except Exception:
            return False


# ── llama.cpp backend ─────────────────────────────────────────────────────────


class LlamaCppService(LocalInferenceBackend):
    """Wraps a llama.cpp server (http://localhost:8080)."""

    def __init__(self, base_url: str | None = None) -> None:
        self.base_url = (base_url or settings.llamacpp_url).rstrip("/")
        self._client: httpx.AsyncClient | None = None

    @property
    def client(self) -> httpx.AsyncClient:
        if self._client is None or self._client.is_closed:
            self._client = httpx.AsyncClient(base_url=self.base_url, timeout=300)
        return self._client

    async def generate(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult:
        full_prompt = f"{system}\n\n{prompt}" if system else prompt
        payload = {
            "prompt": full_prompt,
            "temperature": temperature,
            "n_predict": max_tokens,
            "stream": False,
        }

        async def _do() -> httpx.Response:
            resp = await self.client.post("/completion", json=payload)
            resp.raise_for_status()
            return resp

        resp = await _with_retry(_do)
        data = resp.json()

        return GenerateResult(
            text=data.get("content", ""),
            tokens_used=data.get("tokens_evaluated", 0) + data.get("tokens_predicted", 0),
            model=model,
            duration_ms=data.get("timings", {}).get("predicted_ms", 0),
        )

    async def generate_stream(
        self,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> AsyncIterator[str]:
        full_prompt = f"{system}\n\n{prompt}" if system else prompt
        payload = {
            "prompt": full_prompt,
            "temperature": temperature,
            "n_predict": max_tokens,
            "stream": True,
        }
        async with self.client.stream("POST", "/completion", json=payload) as resp:
            resp.raise_for_status()
            async for line in resp.aiter_lines():
                if not line.strip():
                    continue
                text = line
                if text.startswith("data: "):
                    text = text[6:]
                try:
                    chunk = json.loads(text)
                    token = chunk.get("content", "")
                    if token:
                        yield token
                    if chunk.get("stop", False):
                        return
                except json.JSONDecodeError:
                    continue

    async def chat(
        self,
        model: str,
        messages: list[dict[str, str]],
        *,
        temperature: float = 0.7,
        max_tokens: int = 4096,
    ) -> GenerateResult:
        payload = {
            "model": model,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "stream": False,
        }

        async def _do() -> httpx.Response:
            resp = await self.client.post("/v1/chat/completions", json=payload)
            if resp.status_code >= 400:
                logger.error(
                    "llama.cpp error %d for model %s: %s",
                    resp.status_code, model, resp.text[:500],
                )
            resp.raise_for_status()
            return resp

        resp = await _with_retry(_do)
        data = resp.json()

        choice = data.get("choices", [{}])[0]
        usage = data.get("usage", {})
        return GenerateResult(
            text=choice.get("message", {}).get("content", ""),
            tokens_used=usage.get("total_tokens", 0),
            model=model,
            duration_ms=0,
        )

    async def list_models(self) -> list[dict[str, Any]]:
        """llama.cpp serves a single model; return it via /v1/models."""
        try:
            resp = await self.client.get("/v1/models")
            resp.raise_for_status()
            data = resp.json()
            return [{"name": m.get("id", "unknown"), "size": 0, "modified_at": ""} for m in data.get("data", [])]
        except Exception:
            return []

    async def health(self) -> bool:
        try:
            resp = await self.client.get("/health")
            return resp.status_code == 200
        except Exception:
            return False


# ── Inference router ──────────────────────────────────────────────────────────


class InferenceRouter:
    """Routes inference requests to the correct backend."""

    def __init__(self) -> None:
        self._ollama = OllamaService()
        self._llamacpp = LlamaCppService()

    def get_backend(
        self,
        engine: str,
        base_url: str | None = None,
    ) -> LocalInferenceBackend:
        if engine == "ollama":
            return OllamaService(base_url) if base_url else self._ollama
        if engine == "llamacpp":
            return LlamaCppService(base_url) if base_url else self._llamacpp
        raise ValueError(f"Unknown inference engine: {engine}")

    async def generate(
        self,
        engine: str,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
        base_url: str | None = None,
    ) -> GenerateResult:
        backend = self.get_backend(engine, base_url)
        return await backend.generate(
            model,
            prompt,
            system=system,
            temperature=temperature,
            max_tokens=max_tokens,
        )

    async def generate_stream(
        self,
        engine: str,
        model: str,
        prompt: str,
        *,
        system: str = "",
        temperature: float = 0.7,
        max_tokens: int = 4096,
        base_url: str | None = None,
    ) -> AsyncIterator[str]:
        backend = self.get_backend(engine, base_url)
        async for token in backend.generate_stream(
            model,
            prompt,
            system=system,
            temperature=temperature,
            max_tokens=max_tokens,
        ):
            yield token

    async def chat(
        self,
        engine: str,
        model: str,
        messages: list[dict[str, str]],
        *,
        temperature: float = 0.7,
        max_tokens: int = 4096,
        base_url: str | None = None,
    ) -> GenerateResult:
        backend = self.get_backend(engine, base_url)
        return await backend.chat(
            model,
            messages,
            temperature=temperature,
            max_tokens=max_tokens,
        )

    async def list_models(self, engine: str, base_url: str | None = None) -> list[dict[str, Any]]:
        backend = self.get_backend(engine, base_url)
        return await backend.list_models()

    async def health_check(self, engine: str, base_url: str | None = None) -> bool:
        backend = self.get_backend(engine, base_url)
        return await backend.health()


inference_router = InferenceRouter()
