"""Tests for system stats service."""

import platform

from engine.services.system_stats import get_process_stats, get_system_stats


class TestSystemStats:
    def test_returns_cpu_percent(self):
        stats = get_system_stats()
        assert "cpu_percent" in stats
        assert isinstance(stats["cpu_percent"], (int, float))
        assert 0 <= stats["cpu_percent"] <= 100

    def test_returns_memory_info(self):
        stats = get_system_stats()
        assert stats["memory_percent"] >= 0
        assert stats["memory_total_gb"] > 0
        assert stats["memory_used_gb"] > 0

    def test_returns_platform(self):
        stats = get_system_stats()
        assert stats["platform"] == platform.system()

    def test_returns_cpu_count(self):
        stats = get_system_stats()
        assert stats["cpu_count"] > 0


class TestProcessStats:
    def test_returns_current_process(self):
        stats = get_process_stats()
        assert stats["pid"] > 0
        assert stats["memory_mb"] > 0
        assert stats["threads"] > 0

    def test_handles_invalid_pid(self):
        stats = get_process_stats(pid=999999999)
        assert stats["cpu_percent"] == 0
