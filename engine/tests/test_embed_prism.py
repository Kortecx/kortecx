"""Tests for _embed_prism — rich embedding with metadata, files, and source tracking."""

from __future__ import annotations

from typing import Any
from unittest.mock import MagicMock, patch

import pytest


@pytest.fixture
def mock_hf():
    with patch("engine.routers.experts.hf_service") as mock:
        mock.text_embedding.return_value = [[0.1] * 384]
        yield mock


@pytest.fixture
def mock_qdrant():
    with patch("engine.routers.experts.qdrant_service") as mock:
        collections_mock = MagicMock()
        collections_mock.collections = []
        mock.client.get_collections.return_value = collections_mock
        mock.client.create_collection = MagicMock()
        mock.client.upsert = MagicMock()
        yield mock


class TestEmbedPrism:
    @pytest.mark.asyncio
    async def test_basic_embed(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {
            "id": "exp-test",
            "name": "Test Expert",
            "description": "A test expert for analysis",
            "role": "analyst",
            "category": "research",
            "tags": ["data", "analysis"],
            "capabilities": ["reasoning", "analysis"],
        }
        await _embed_prism(expert)

        # Verify embedding was called with rich text
        call_args = mock_hf.text_embedding.call_args
        text = call_args[0][1]
        assert "Test Expert" in text
        assert "A test expert for analysis" in text
        assert "Role: analyst" in text
        assert "data" in text
        assert "reasoning" in text

        # Verify upsert was called with correct payload
        upsert_call = mock_qdrant.client.upsert.call_args
        point = upsert_call[1]["points"][0]
        assert point.payload["expert_id"] == "exp-test"
        assert point.payload["source"] == "local"
        assert point.payload["description"] == "A test expert for analysis"
        assert point.payload["has_files"] is False

    @pytest.mark.asyncio
    async def test_embed_with_system_prompt(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {
            "id": "exp-prompt",
            "name": "Prompt Expert",
            "description": "Expert with prompt",
            "systemPrompt": "You are a deep research analyst specializing in data science",
            "role": "researcher",
        }
        await _embed_prism(expert)

        text = mock_hf.text_embedding.call_args[0][1]
        assert "deep research analyst" in text
        assert "data science" in text

    @pytest.mark.asyncio
    async def test_embed_with_file_texts(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {"id": "exp-files", "name": "File Expert", "role": "coder"}
        file_texts = ["README: This project handles ETL pipelines", "Config: database connection settings"]
        await _embed_prism(expert, file_texts=file_texts)

        text = mock_hf.text_embedding.call_args[0][1]
        assert "ETL pipelines" in text
        assert "database connection" in text

        point = mock_qdrant.client.upsert.call_args[1]["points"][0]
        assert point.payload["has_files"] is True

    @pytest.mark.asyncio
    async def test_embed_with_marketplace_source(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {"id": "mp-test", "name": "Marketplace Expert", "role": "writer"}
        await _embed_prism(expert, source="marketplace")

        point = mock_qdrant.client.upsert.call_args[1]["points"][0]
        assert point.payload["source"] == "marketplace"

    @pytest.mark.asyncio
    async def test_embed_with_specializations(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {
            "id": "exp-spec",
            "name": "Specialist",
            "role": "analyst",
            "specializations": ["Deep Research", "Data Analysis", "Machine Learning"],
        }
        await _embed_prism(expert)

        text = mock_hf.text_embedding.call_args[0][1]
        assert "Deep Research" in text
        assert "Machine Learning" in text

    @pytest.mark.asyncio
    async def test_file_texts_truncated(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {"id": "exp-trunc", "name": "Trunc", "role": "coder"}
        long_text = "q" * 1000
        await _embed_prism(expert, file_texts=[long_text])

        text = mock_hf.text_embedding.call_args[0][1]
        # File text should be truncated to 500 chars (not the full 1000)
        assert text.count("q") <= 500
        assert text.count("q") > 0
        # Full text would be much longer if not truncated
        assert len(text) < 1000

    @pytest.mark.asyncio
    async def test_max_five_files(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        expert = {"id": "exp-max", "name": "Max Files Expert", "role": "coder"}
        file_texts = [f"file-{i}-content" for i in range(10)]
        await _embed_prism(expert, file_texts=file_texts)

        text = mock_hf.text_embedding.call_args[0][1]
        # Only first 5 files should be included
        assert "file-4-content" in text
        assert "file-5-content" not in text

    @pytest.mark.asyncio
    async def test_embed_fails_gracefully(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        mock_hf.text_embedding.return_value = []
        expert = {"id": "exp-empty", "name": "Empty", "role": "coder"}
        # Should not raise
        await _embed_prism(expert)
        mock_qdrant.client.upsert.assert_not_called()


class TestEmbedPrismErrorClassification:
    """Verify that _embed_prism logs specific error types, not generic messages."""

    @pytest.mark.asyncio
    async def test_logs_auth_error_clearly(self, mock_hf: Any, mock_qdrant: Any, caplog: Any) -> None:
        from engine.routers.experts import _embed_prism

        mock_hf.text_embedding.side_effect = Exception("401 Unauthorized for url")
        expert = {"id": "exp-auth", "name": "Auth Fail", "role": "coder"}

        await _embed_prism(expert)

        assert any("authentication failed" in r.message.lower() for r in caplog.records)

    @pytest.mark.asyncio
    async def test_logs_network_error_clearly(self, mock_hf: Any, mock_qdrant: Any, caplog: Any) -> None:
        from engine.routers.experts import _embed_prism

        mock_hf.text_embedding.side_effect = ConnectionError("Connection refused")
        expert = {"id": "exp-net", "name": "Net Fail", "role": "coder"}

        await _embed_prism(expert)

        assert any("network error" in r.message.lower() for r in caplog.records)

    @pytest.mark.asyncio
    async def test_logs_qdrant_error_clearly(self, mock_hf: Any, mock_qdrant: Any, caplog: Any) -> None:
        from engine.routers.experts import _embed_prism

        mock_qdrant.client.get_collections.side_effect = Exception("Collection not found")
        expert = {"id": "exp-qd", "name": "Qdrant Fail", "role": "coder"}

        await _embed_prism(expert)

        assert any("qdrant collection error" in r.message.lower() for r in caplog.records)

    @pytest.mark.asyncio
    async def test_startup_no_crash_when_nothing_available(self, mock_hf: Any, mock_qdrant: Any) -> None:
        from engine.routers.experts import _embed_prism

        mock_hf.text_embedding.return_value = []
        expert = {"id": "exp-skip", "name": "Skipped", "role": "coder"}

        # Should not raise, just skip
        await _embed_prism(expert)
        mock_qdrant.client.upsert.assert_not_called()
