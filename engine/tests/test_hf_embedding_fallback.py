"""Tests for HuggingFaceService.text_embedding local fallback logic."""

from __future__ import annotations

from unittest.mock import MagicMock, patch

import numpy as np

from engine.services.hf import HuggingFaceService


class TestTextEmbeddingLocalFallback:
    """Verify local sentence-transformers is tried first, then HF API."""

    def _make_service(self, *, token: str = "") -> HuggingFaceService:
        with patch("engine.services.hf.settings") as mock_settings:
            mock_settings.hf_token = token
            from engine.services.hf import HuggingFaceService

            return HuggingFaceService()

    def test_local_model_used_when_available(self) -> None:
        svc = self._make_service()
        mock_model = MagicMock()
        mock_model.encode.return_value = np.array([[0.1, 0.2, 0.3]])

        with patch("sentence_transformers.SentenceTransformer", return_value=mock_model):
            result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        mock_model.encode.assert_called_once()
        assert result == [[0.1, 0.2, 0.3]]

    def test_falls_back_to_api_when_local_unavailable(self) -> None:
        svc = self._make_service(token="hf_test_token")
        svc._local_available = False  # simulate sentence-transformers not installed

        mock_inference = MagicMock()
        mock_inference.feature_extraction.return_value = [[0.4, 0.5, 0.6]]
        svc._inference = mock_inference

        result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        mock_inference.feature_extraction.assert_called_once()
        assert result == [[0.4, 0.5, 0.6]]

    def test_returns_empty_when_neither_available(self) -> None:
        svc = self._make_service(token="")
        svc._local_available = False

        result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        assert result == []

    def test_local_model_cached_after_first_load(self) -> None:
        svc = self._make_service()
        mock_model = MagicMock()
        mock_model.encode.return_value = np.array([[0.1, 0.2]])
        mock_constructor = MagicMock(return_value=mock_model)

        with patch("sentence_transformers.SentenceTransformer", mock_constructor):
            svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "a")
            svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "b")

        # Constructor called only once, model reused
        mock_constructor.assert_called_once()
        assert mock_model.encode.call_count == 2

    def test_api_fallback_on_local_encode_failure(self) -> None:
        svc = self._make_service(token="hf_test_token")
        mock_model = MagicMock()
        mock_model.encode.side_effect = RuntimeError("CUDA OOM")

        mock_inference = MagicMock()
        mock_inference.feature_extraction.return_value = [[0.7, 0.8]]
        svc._inference = mock_inference

        with patch("sentence_transformers.SentenceTransformer", return_value=mock_model):
            result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "test")

        assert result == [[0.7, 0.8]]
        mock_inference.feature_extraction.assert_called_once()

    def test_returns_correct_shape_single_string(self) -> None:
        svc = self._make_service()
        mock_model = MagicMock()
        mock_model.encode.return_value = np.array([[0.1, 0.2, 0.3]])

        with patch("sentence_transformers.SentenceTransformer", return_value=mock_model):
            result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        assert isinstance(result, list)
        assert isinstance(result[0], list)
        assert len(result) == 1

    def test_returns_correct_shape_batch(self) -> None:
        svc = self._make_service()
        mock_model = MagicMock()
        mock_model.encode.return_value = np.array([[0.1, 0.2], [0.3, 0.4]])

        with patch("sentence_transformers.SentenceTransformer", return_value=mock_model):
            result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", ["a", "b"])

        assert len(result) == 2
        assert result[0] == [0.1, 0.2]
        assert result[1] == [0.3, 0.4]

    def test_api_wraps_single_float_list(self) -> None:
        """HF API sometimes returns flat list for single input."""
        svc = self._make_service(token="hf_test_token")
        svc._local_available = False

        mock_inference = MagicMock()
        mock_inference.feature_extraction.return_value = [0.1, 0.2, 0.3]
        svc._inference = mock_inference

        result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        assert result == [[0.1, 0.2, 0.3]]

    def test_api_error_returns_empty(self) -> None:
        svc = self._make_service(token="hf_bad_token")
        svc._local_available = False

        mock_inference = MagicMock()
        mock_inference.feature_extraction.side_effect = Exception("401 Unauthorized")
        svc._inference = mock_inference

        result = svc.text_embedding("sentence-transformers/all-MiniLM-L6-v2", "hello")

        assert result == []
