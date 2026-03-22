"""Tests for engine configuration."""

from __future__ import annotations

import pytest


class TestSettings:
    def test_defaults(self, monkeypatch):
        # Clear env overrides so we test actual defaults
        monkeypatch.delenv("DATABASE_URL", raising=False)
        monkeypatch.delenv("HOST", raising=False)
        monkeypatch.delenv("PORT", raising=False)
        from engine.config import Settings

        s = Settings()
        assert s.port == 8000
        assert s.host == "0.0.0.0"
        assert "kortecx" in s.database_url
        assert s.ollama_url.startswith("http")
        assert s.llamacpp_url.startswith("http")

    def test_quorum_defaults(self):
        from engine.config import Settings

        s = Settings()
        assert s.quorum_max_concurrent == 4
        assert s.quorum_metrics_interval == 5.0
        assert s.quorum_default_workers == 3
        assert s.quorum_default_retries == 3

    def test_agent_defaults(self):
        from engine.config import Settings

        s = Settings()
        assert s.max_concurrent_agents == 10
        assert s.agent_retry_enabled is True
        assert s.default_local_engine in ("ollama", "llamacpp")

    def test_override_via_env(self, monkeypatch):
        monkeypatch.setenv("PORT", "9999")
        from engine.config import Settings

        s = Settings()
        assert s.port == 9999
