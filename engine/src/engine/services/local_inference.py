"""Local inference backends — Ollama and llama.cpp server wrappers."""

from __future__ import annotations

import logging
from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Any

import httpx

from engine.config import settings

logger = logging.getLogger("engine.local_inference")


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


class OllamaService(LocalInferenceBackend):
    """Wraps the Ollama REST API (http://localhost:11434)."""

    def __init__(self, base_url: str | None = None) -> None:
        self.base_url = (base_url or settings.ollama_url).rstrip("/")

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
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        }
        if system:
            payload["system"] = system

        async with httpx.AsyncClient(timeout=300) as client:
            resp = await client.post(f"{self.base_url}/api/generate", json=payload)
            resp.raise_for_status()
            data = resp.json()

        return GenerateResult(
            text=data.get("response", ""),
            tokens_used=data.get("eval_count", 0) + data.get("prompt_eval_count", 0),
            model=model,
            duration_ms=data.get("total_duration", 0) / 1_000_000,  # ns → ms
        )

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
                "temperature": temperature,
                "num_predict": max_tokens,
            },
        }

        async with httpx.AsyncClient(timeout=300) as client:
            resp = await client.post(f"{self.base_url}/api/chat", json=payload)
            resp.raise_for_status()
            data = resp.json()

        msg = data.get("message", {})
        return GenerateResult(
            text=msg.get("content", ""),
            tokens_used=data.get("eval_count", 0) + data.get("prompt_eval_count", 0),
            model=model,
            duration_ms=data.get("total_duration", 0) / 1_000_000,
        )

    async def list_models(self) -> list[dict[str, Any]]:
        async with httpx.AsyncClient(timeout=10) as client:
            resp = await client.get(f"{self.base_url}/api/tags")
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
        async with httpx.AsyncClient(timeout=600) as client:
            resp = await client.post(
                f"{self.base_url}/api/pull",
                json={"name": model, "stream": False},
            )
            resp.raise_for_status()

    async def health(self) -> bool:
        try:
            async with httpx.AsyncClient(timeout=5) as client:
                resp = await client.get(self.base_url)
                return resp.status_code == 200
        except Exception:
            return False


class LlamaCppService(LocalInferenceBackend):
    """Wraps a llama.cpp server (http://localhost:8080)."""

    def __init__(self, base_url: str | None = None) -> None:
        self.base_url = (base_url or settings.llamacpp_url).rstrip("/")

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

        async with httpx.AsyncClient(timeout=300) as client:
            resp = await client.post(f"{self.base_url}/completion", json=payload)
            resp.raise_for_status()
            data = resp.json()

        return GenerateResult(
            text=data.get("content", ""),
            tokens_used=data.get("tokens_evaluated", 0) + data.get("tokens_predicted", 0),
            model=model,
            duration_ms=data.get("timings", {}).get("predicted_ms", 0),
        )

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

        async with httpx.AsyncClient(timeout=300) as client:
            resp = await client.post(
                f"{self.base_url}/v1/chat/completions", json=payload,
            )
            resp.raise_for_status()
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
            async with httpx.AsyncClient(timeout=5) as client:
                resp = await client.get(f"{self.base_url}/v1/models")
                resp.raise_for_status()
                data = resp.json()
            return [
                {"name": m.get("id", "unknown"), "size": 0, "modified_at": ""}
                for m in data.get("data", [])
            ]
        except Exception:
            return []

    async def health(self) -> bool:
        try:
            async with httpx.AsyncClient(timeout=5) as client:
                resp = await client.get(f"{self.base_url}/health")
                return resp.status_code == 200
        except Exception:
            return False


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
            model, prompt, system=system,
            temperature=temperature, max_tokens=max_tokens,
        )

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
            model, messages,
            temperature=temperature, max_tokens=max_tokens,
        )

    async def list_models(self, engine: str, base_url: str | None = None) -> list[dict[str, Any]]:
        backend = self.get_backend(engine, base_url)
        return await backend.list_models()

    async def health_check(self, engine: str, base_url: str | None = None) -> bool:
        backend = self.get_backend(engine, base_url)
        return await backend.health()


inference_router = InferenceRouter()
