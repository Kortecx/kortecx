"""Tests for local inference infrastructure (no server required)."""

import pytest
from engine.services.local_inference import (
    GenerateResult,
    InferenceRouter,
    ModelPool,
    OllamaService,
    LlamaCppService,
)


class TestGenerateResult:
    def test_dataclass_fields(self):
        r = GenerateResult(text="hello", tokens_used=10, model="test", duration_ms=100.0)
        assert r.text == "hello"
        assert r.tokens_used == 10
        assert r.model == "test"
        assert r.duration_ms == 100.0


class TestInferenceRouter:
    def test_get_ollama_backend(self):
        router = InferenceRouter()
        backend = router.get_backend("ollama")
        assert isinstance(backend, OllamaService)

    def test_get_llamacpp_backend(self):
        router = InferenceRouter()
        backend = router.get_backend("llamacpp")
        assert isinstance(backend, LlamaCppService)

    def test_unknown_engine_raises(self):
        router = InferenceRouter()
        with pytest.raises(ValueError, match="Unknown inference engine"):
            router.get_backend("unknown")

    def test_custom_base_url(self):
        router = InferenceRouter()
        backend = router.get_backend("ollama", base_url="http://custom:11434")
        assert isinstance(backend, OllamaService)
        assert backend.base_url == "http://custom:11434"


class TestModelPool:
    @pytest.mark.asyncio
    async def test_acquire_release(self):
        pool = ModelPool()
        await pool.acquire("model-a")
        assert pool.active_models == {"model-a": 1}
        assert pool.total_active == 1

        await pool.acquire("model-a")
        assert pool.active_models == {"model-a": 2}

        await pool.release("model-a")
        assert pool.active_models == {"model-a": 1}

        await pool.release("model-a")
        assert pool.active_models == {}
        assert pool.total_active == 0

    @pytest.mark.asyncio
    async def test_multiple_models(self):
        pool = ModelPool()
        await pool.acquire("model-a")
        await pool.acquire("model-b")
        assert pool.total_active == 2
        assert set(pool.active_models.keys()) == {"model-a", "model-b"}
