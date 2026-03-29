"""Tests for plan_store filesystem service."""

import json
from pathlib import Path

import pytest

from engine.services import plan_store


@pytest.fixture(autouse=True)
def clean_plans(tmp_path, monkeypatch):
    """Redirect plan dirs to tmp and clean up."""
    live = tmp_path / "LIVE"
    freeze = tmp_path / "FREEZE"
    live.mkdir()
    freeze.mkdir()
    monkeypatch.setattr(plan_store, "LIVE_DIR", live)
    monkeypatch.setattr(plan_store, "FREEZE_DIR", freeze)
    yield


@pytest.fixture
def sample_dag():
    return {
        "nodes": [
            {"id": "n1", "label": "Research", "prismId": "researcher"},
            {"id": "n2", "label": "Analyze", "prismId": "analyst"},
        ],
        "edges": [{"id": "e1", "source": "n1", "target": "n2"}],
    }


class TestSaveLivePlan:
    def test_saves_json_and_md(self, sample_dag):
        result = plan_store.save_live_plan("test-wf", sample_dag, markdown="# Plan")
        assert result["version"] == 1
        path = Path(result["path"])
        assert path.exists()
        data = json.loads(path.read_text())
        assert len(data["nodes"]) == 2

        md_path = path.with_suffix(".md")
        assert md_path.exists()
        assert md_path.read_text() == "# Plan"

    def test_increments_version(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag)
        result = plan_store.save_live_plan("test-wf", sample_dag)
        assert result["version"] == 2

    def test_prunes_old_versions(self, sample_dag):
        for _ in range(5):
            plan_store.save_live_plan("test-wf", sample_dag, max_versions=3)

        versions = plan_store.list_live_versions("test-wf")
        assert len(versions) == 3
        # Newest should be v5
        assert versions[0]["version"] == 5


class TestGetLivePlan:
    def test_returns_none_for_missing(self):
        assert plan_store.get_live_plan("nonexistent") is None

    def test_returns_latest(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v1")
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v2")
        plan = plan_store.get_live_plan("test-wf")
        assert plan is not None
        assert plan["version"] == 2
        assert plan["markdown"] == "v2"


class TestGetLivePlanVersion:
    def test_specific_version(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v1")
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v2")
        plan = plan_store.get_live_plan_version("test-wf", 1)
        assert plan is not None
        assert plan["version"] == 1
        assert plan["markdown"] == "v1"

    def test_missing_version(self, sample_dag):
        assert plan_store.get_live_plan_version("test-wf", 99) is None


class TestFreezePlan:
    def test_freeze_copies_live(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="# Live")
        result = plan_store.freeze_plan("test-wf")
        assert result is not None
        assert result["markdown"] == "# Live"

        frozen = plan_store.get_frozen_plan("test-wf")
        assert frozen is not None
        assert len(frozen["dag"]["nodes"]) == 2

    def test_freeze_no_live(self):
        result = plan_store.freeze_plan("empty-wf")
        assert result is None

    def test_unfreeze_removes(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag)
        plan_store.freeze_plan("test-wf")
        assert plan_store.get_frozen_plan("test-wf") is not None

        plan_store.unfreeze_plan("test-wf")
        assert plan_store.get_frozen_plan("test-wf") is None

    def test_refreeze(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v1")
        plan_store.save_live_plan("test-wf", sample_dag, markdown="v2")
        plan_store.freeze_plan("test-wf")

        # Refreeze with v1
        result = plan_store.refreeze_plan("test-wf", version=1)
        assert result is not None
        assert result["markdown"] == "v1"


class TestGetExecutionPlan:
    def test_frozen_returns_freeze(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="live")
        plan_store.freeze_plan("test-wf")
        plan = plan_store.get_execution_plan("test-wf", is_frozen=True)
        assert plan is not None

    def test_unfrozen_returns_live(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="live")
        plan = plan_store.get_execution_plan("test-wf", is_frozen=False)
        assert plan is not None
        assert plan["markdown"] == "live"

    def test_frozen_fallback_to_live(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag, markdown="live")
        # No freeze dir, but is_frozen=True → falls back to live
        plan = plan_store.get_execution_plan("test-wf", is_frozen=True)
        assert plan is not None


class TestListLiveVersions:
    def test_empty(self):
        assert plan_store.list_live_versions("nonexistent") == []

    def test_lists_all(self, sample_dag):
        plan_store.save_live_plan("test-wf", sample_dag)
        plan_store.save_live_plan("test-wf", sample_dag)
        versions = plan_store.list_live_versions("test-wf")
        assert len(versions) == 2
        assert versions[0]["version"] == 2  # newest first
