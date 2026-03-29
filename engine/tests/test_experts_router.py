"""Tests for experts router — endpoints, request models, helper functions."""

from __future__ import annotations

from datetime import UTC, datetime
from unittest.mock import AsyncMock, patch

import pytest

from engine.routers.experts import (
    AttachRequest,
    CreateExpertRequest,
    ExecuteExpertRequest,
    RestoreVersionRequest,
    SaveRunArtifactRequest,
    UpdateFileRequest,
    _clean,
    _cleanup_old_runs,
    _expert_runs,
    router,
)

# ── Request model tests ─────────────────────────────────────────────────────


class TestRequestModels:
    def test_create_expert_defaults(self):
        req = CreateExpertRequest(name="Test", role="coder")
        assert req.name == "Test"
        assert req.role == "coder"
        assert req.description == ""
        assert req.systemPrompt == ""
        assert req.modelSource == "local"
        assert req.temperature == 0.7
        assert req.maxTokens == 4096
        assert req.tags == []
        assert req.capabilities == []
        assert req.isPublic is False
        assert req.category == "custom"
        assert req.complexityLevel == 3

    def test_create_expert_full(self):
        req = CreateExpertRequest(
            name="Analyst",
            role="analyst",
            description="Analyzes data",
            systemPrompt="You are an analyst",
            userPrompt="Analyze {{input}}",
            modelSource="provider",
            localModelConfig={"engine": "ollama", "modelName": "llama3.2:3b"},
            temperature=0.5,
            maxTokens=8192,
            tags=["data", "analysis"],
            capabilities=["analysis", "reasoning"],
            isPublic=True,
            category="engineering",
            complexityLevel=5,
        )
        assert req.description == "Analyzes data"
        assert req.isPublic is True
        assert req.complexityLevel == 5

    def test_execute_expert_defaults(self):
        req = ExecuteExpertRequest(expertName="test-expert")
        assert req.model == "llama3.2:3b"
        assert req.engine == "ollama"
        assert req.temperature == 0.7
        assert req.maxTokens == 4096
        assert req.callbackUrl is None

    def test_save_run_artifact_defaults(self):
        req = SaveRunArtifactRequest(expertName="test", response="Hello world")
        assert req.tokensUsed == 0
        assert req.durationMs == 0
        assert req.tags == []
        assert req.metadata is None

    def test_update_file_request(self):
        req = UpdateFileRequest(filename="system.md", content="New prompt")
        assert req.filename == "system.md"
        assert req.content == "New prompt"

    def test_restore_version_request(self):
        req = RestoreVersionRequest(version="system.md.v1")
        assert req.version == "system.md.v1"

    def test_attach_request(self):
        req = AttachRequest(targetId="expert-123")
        assert req.targetId == "expert-123"


# ── Helper function tests ────────────────────────────────────────────────────


class TestCleanHelper:
    def test_strips_internal_fields(self):
        data = {"id": "e1", "name": "Test", "_source": "marketplace", "_dir": "/tmp/e1"}
        cleaned = _clean(data)
        assert "id" in cleaned
        assert "name" in cleaned
        assert "_source" not in cleaned
        assert "_dir" not in cleaned

    def test_preserves_public_fields(self):
        data = {"id": "e1", "role": "coder", "tags": ["a", "b"]}
        cleaned = _clean(data)
        assert cleaned == data

    def test_empty_dict(self):
        assert _clean({}) == {}


class TestCleanupOldRuns:
    def setup_method(self):
        _expert_runs.clear()

    def teardown_method(self):
        _expert_runs.clear()

    def test_removes_old_completed_runs(self):
        old_time = datetime(2020, 1, 1, tzinfo=UTC).isoformat()
        _expert_runs["old-run"] = {
            "status": "completed",
            "completedAt": old_time,
        }
        removed = _cleanup_old_runs()
        assert removed == 1
        assert "old-run" not in _expert_runs

    def test_keeps_recent_completed_runs(self):
        recent_time = datetime.now(UTC).isoformat()
        _expert_runs["recent-run"] = {
            "status": "completed",
            "completedAt": recent_time,
        }
        removed = _cleanup_old_runs()
        assert removed == 0
        assert "recent-run" in _expert_runs

    def test_keeps_running_runs(self):
        _expert_runs["running"] = {"status": "running"}
        removed = _cleanup_old_runs()
        assert removed == 0
        assert "running" in _expert_runs

    def test_removes_old_failed_runs(self):
        old_time = datetime(2020, 1, 1, tzinfo=UTC).isoformat()
        _expert_runs["failed-run"] = {
            "status": "failed",
            "completedAt": old_time,
        }
        removed = _cleanup_old_runs()
        assert removed == 1

    def test_removes_runs_with_invalid_timestamp(self):
        _expert_runs["bad-ts"] = {
            "status": "completed",
            "completedAt": "not-a-date",
        }
        removed = _cleanup_old_runs()
        assert removed == 1

    def test_empty_runs(self):
        assert _cleanup_old_runs() == 0


# ── Endpoint tests (using FastAPI TestClient) ────────────────────────────────


@pytest.fixture
def mock_expert_manager():
    with patch("engine.routers.experts.expert_manager") as mock:
        mock.load_all.return_value = [
            {
                "id": "exp-1",
                "name": "Alpha",
                "role": "coder",
                "_source": "marketplace",
                "_dir": "/tmp/alpha",
            },
            {
                "id": "exp-2",
                "name": "Beta",
                "role": "analyst",
                "_source": "local",
                "_dir": "/tmp/beta",
            },
        ]
        mock.get.return_value = {
            "id": "exp-1",
            "name": "Alpha",
            "role": "coder",
            "_source": "marketplace",
            "_dir": "/tmp/alpha",
        }
        mock.get_prompt.return_value = "You are an alpha expert."
        mock.list_files.return_value = ["expert.json", "system.md", "user.md"]
        mock.create_local.return_value = {
            "id": "new-1",
            "name": "NewExpert",
            "role": "writer",
            "_source": "local",
            "_dir": "/tmp/new",
        }
        mock.update_file.return_value = {"ok": True, "version": "system.md.v1"}
        mock.get_versions.return_value = [{"version": "v1", "timestamp": "2025-01-01T00:00:00Z"}]
        mock.restore_version.return_value = {"ok": True, "restored": "system.md.v1"}
        mock.delete_expert.return_value = True
        mock.sync_all_to_db = AsyncMock(return_value={"synced": 2, "errors": 0})
        yield mock


@pytest.fixture
def mock_expert_artifacts():
    with patch("engine.routers.experts.expert_artifacts") as mock:
        mock.list_artifacts.return_value = [
            {"id": "art-1", "expertName": "Alpha", "fileType": "md"},
        ]
        mock.save_response.return_value = {"saved": True, "path": "/tmp/art.md"}
        yield mock


@pytest.fixture
def mock_embed():
    with patch("engine.routers.experts._embed_prism", new_callable=AsyncMock) as mock:
        yield mock


@pytest.fixture
def mock_qdrant():
    with patch("engine.routers.experts.qdrant_service") as mock:
        mock.delete = AsyncMock()
        yield mock


@pytest.fixture
def client(mock_expert_manager, mock_expert_artifacts, mock_embed, mock_qdrant):
    from fastapi import FastAPI
    from fastapi.testclient import TestClient

    app = FastAPI()
    app.include_router(router, prefix="/experts")
    return TestClient(app)


class TestListExperts:
    def test_list_returns_marketplace_and_local(self, client):
        resp = client.get("/experts/list")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 2
        assert len(data["marketplace"]) == 1
        assert len(data["local"]) == 1
        # Verify internal fields are stripped
        for e in data["marketplace"] + data["local"]:
            assert "_source" not in e
            assert "_dir" not in e


class TestGetExpert:
    def test_get_existing(self, client):
        resp = client.get("/experts/exp-1")
        assert resp.status_code == 200
        data = resp.json()
        assert data["id"] == "exp-1"
        assert "systemPrompt" in data
        assert "userPrompt" not in data or isinstance(data.get("userPrompt"), str)
        assert "files" in data

    def test_get_nonexistent(self, client, mock_expert_manager):
        mock_expert_manager.get.return_value = None
        resp = client.get("/experts/nonexistent")
        data = resp.json()
        assert "error" in data


class TestCreateExpert:
    def test_create_success(self, client):
        resp = client.post("/experts/create", json={"name": "New", "role": "writer"})
        assert resp.status_code == 200
        data = resp.json()
        assert "expert" in data
        assert data["expert"]["name"] == "NewExpert"
        assert "_source" not in data["expert"]

    def test_create_with_full_config(self, client):
        resp = client.post(
            "/experts/create",
            json={
                "name": "Full",
                "role": "analyst",
                "description": "Full expert",
                "systemPrompt": "Be analytical",
                "tags": ["data"],
                "complexityLevel": 5,
            },
        )
        assert resp.status_code == 200


class TestUpdateExpertFile:
    def test_update_success(self, client):
        resp = client.post(
            "/experts/exp-1/update",
            json={"filename": "system.md", "content": "Updated prompt"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True

    def test_update_error(self, client, mock_expert_manager):
        mock_expert_manager.update_file.side_effect = ValueError("File not found")
        resp = client.post(
            "/experts/exp-1/update",
            json={"filename": "missing.md", "content": "x"},
        )
        data = resp.json()
        assert "error" in data


class TestVersions:
    def test_list_versions(self, client):
        resp = client.get("/experts/exp-1/versions/system.md")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 1
        assert len(data["versions"]) == 1

    def test_restore_version(self, client):
        resp = client.post("/experts/exp-1/restore", json={"version": "system.md.v1"})
        assert resp.status_code == 200

    def test_restore_empty_version(self, client):
        resp = client.post("/experts/exp-1/restore", json={"version": ""})
        data = resp.json()
        assert "error" in data


class TestDeleteExpert:
    def test_delete_success(self, client):
        resp = client.delete("/experts/exp-1")
        assert resp.status_code == 200
        data = resp.json()
        assert data["deleted"] is True
        assert data["id"] == "exp-1"

    def test_delete_error(self, client, mock_expert_manager):
        mock_expert_manager.delete_expert.side_effect = ValueError("Cannot delete marketplace expert")
        resp = client.delete("/experts/exp-1")
        data = resp.json()
        assert "error" in data


class TestExecuteExpert:
    def setup_method(self):
        _expert_runs.clear()

    def teardown_method(self):
        _expert_runs.clear()

    def test_execute_returns_run_id(self, client):
        resp = client.post(
            "/experts/exp-1/execute",
            json={"expertName": "Alpha"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert "runId" in data
        assert data["status"] == "started"
        assert data["runId"].startswith("er-")

    def test_get_run_status(self, client):
        _expert_runs["er-test123"] = {
            "runId": "er-test123",
            "expertId": "exp-1",
            "status": "completed",
            "responseText": "Hello",
        }
        resp = client.get("/experts/exp-1/execute/er-test123")
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "completed"

    def test_get_run_not_found(self, client):
        resp = client.get("/experts/exp-1/execute/er-nonexistent")
        data = resp.json()
        assert "error" in data


class TestArtifacts:
    def test_list_all_artifacts(self, client):
        resp = client.get("/experts/artifacts/all")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 1

    def test_list_all_with_type_filter(self, client, mock_expert_artifacts):
        mock_expert_artifacts.list_artifacts.return_value = [
            {"id": "a1", "fileType": "md"},
            {"id": "a2", "fileType": "json"},
        ]
        resp = client.get("/experts/artifacts/all?file_type=md")
        data = resp.json()
        assert data["total"] == 1

    def test_save_run_artifact(self, client):
        resp = client.post(
            "/experts/exp-1/run-artifact",
            json={"expertName": "Alpha", "response": "Test output"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["saved"] is True

    def test_list_expert_artifacts(self, client):
        resp = client.get("/experts/exp-1/artifacts")
        assert resp.status_code == 200
        data = resp.json()
        assert "artifacts" in data


class TestListFiles:
    def test_list_expert_files(self, client):
        resp = client.get("/experts/exp-1/files")
        assert resp.status_code == 200
        data = resp.json()
        assert data["total"] == 3
        assert "expert.json" in data["files"]


class TestGetPrompt:
    def test_get_system_prompt(self, client):
        resp = client.get("/experts/exp-1/prompt/system")
        assert resp.status_code == 200
        data = resp.json()
        assert data["type"] == "system"
        assert "content" in data


class TestEmbedEndpoints:
    def test_embed_prism(self, client):
        resp = client.post("/experts/exp-1/embed")
        assert resp.status_code == 200
        data = resp.json()
        assert data["embedded"] is True

    def test_embed_nonexistent(self, client, mock_expert_manager):
        mock_expert_manager.get.return_value = None
        resp = client.post("/experts/nonexistent/embed")
        data = resp.json()
        assert "error" in data

    def test_embed_all(self, client):
        resp = client.post("/experts/embed/all")
        assert resp.status_code == 200
        data = resp.json()
        assert "embedded" in data
        assert "errors" in data


class TestAttach:
    def test_attach_success(self, client, mock_expert_manager):
        mock_expert_manager.get.side_effect = lambda eid: {
            "exp-1": {"id": "exp-1", "name": "Alpha", "description": "A", "tags": ["a"], "_source": "local"},
            "exp-2": {"id": "exp-2", "name": "Beta", "description": "B", "tags": ["b"], "_source": "local"},
        }.get(eid)
        resp = client.post("/experts/exp-1/attach", json={"targetId": "exp-2"})
        assert resp.status_code == 200
        data = resp.json()
        assert data["attached"] is True

    def test_attach_missing_source(self, client, mock_expert_manager):
        mock_expert_manager.get.return_value = None
        resp = client.post("/experts/missing/attach", json={"targetId": "exp-2"})
        data = resp.json()
        assert "error" in data

    def test_attach_missing_target(self, client, mock_expert_manager):
        mock_expert_manager.get.side_effect = lambda eid: {"id": "exp-1", "name": "Alpha", "_source": "local"} if eid == "exp-1" else None
        resp = client.post("/experts/exp-1/attach", json={"targetId": "missing"})
        data = resp.json()
        assert "error" in data


class TestEmbedAssets:
    def test_embed_assets_success(self, client):
        resp = client.post(
            "/experts/exp-1/embed-assets",
            json={"file_texts": ["document about data analysis", "readme for pipeline"]},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["embedded"] is True
        assert data["fileCount"] == 2

    def test_embed_assets_nonexistent(self, client, mock_expert_manager):
        mock_expert_manager.get.return_value = None
        resp = client.post(
            "/experts/nonexistent/embed-assets",
            json={"file_texts": ["some text"]},
        )
        data = resp.json()
        assert "error" in data


class TestEmbedBulk:
    def test_embed_bulk_success(self, client):
        experts_data = [
            {"id": "mp-1", "name": "Expert A", "description": "A specialist", "role": "researcher"},
            {"id": "mp-2", "name": "Expert B", "description": "B specialist", "role": "coder"},
        ]
        resp = client.post(
            "/experts/embed/bulk",
            json={"experts": experts_data, "source": "marketplace"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["embedded"] == 2
        assert data["errors"] == 0
        assert data["total"] == 2

    def test_embed_bulk_empty(self, client):
        resp = client.post(
            "/experts/embed/bulk",
            json={"experts": [], "source": "marketplace"},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["embedded"] == 0
