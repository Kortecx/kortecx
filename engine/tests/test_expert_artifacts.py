"""Tests for expert artifacts — date-based disk persistence and script extraction."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import engine.services.expert_artifacts as ea


@pytest.fixture
def temp_artifacts(tmp_path, monkeypatch):
    monkeypatch.setattr(ea, "EXPERTS_ROOT", tmp_path)
    return ea.ExpertArtifacts()


class TestExpertArtifacts:
    def test_get_artifact_dir_creates_date_directory(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        d = artifacts.get_artifact_dir("Research Analyst", "2026-03-22")
        assert d.exists()
        assert "2026-03-22" in str(d)
        assert "research-analyst" in str(d)

    def test_get_artifact_dir_defaults_to_today(self, temp_artifacts):
        artifacts = temp_artifacts
        d = artifacts.get_artifact_dir("Test Expert")
        assert d.exists()
        # Should contain a date-like directory
        parts = d.parts
        assert any(len(p) == 10 and p.count("-") == 2 for p in parts)

    def test_save_response_creates_all_files(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        result = artifacts.save_response(
            expert_id="exp-123",
            expert_name="Data Engineer",
            response="Here is the analysis result.",
            prompt="Analyze the data",
            system_prompt="You are a data engineer.",
            model="llama3.2:3b",
            engine="ollama",
            tokens_used=200,
            duration_ms=1500,
            tags=["data", "analysis"],
        )

        assert "files" in result
        assert "runId" in result
        assert "date" in result
        assert len(result["files"]) >= 4  # response, context, prompt, system

        # Verify response file
        response_files = [f for f in result["files"] if f["category"] == "response"]
        assert len(response_files) == 1
        response_path = Path(response_files[0]["filePath"])
        assert response_path.exists()
        content = response_path.read_text()
        assert "Here is the analysis result." in content
        assert "expert: Data Engineer" in content  # frontmatter

        # Verify context file
        context_files = [f for f in result["files"] if f["category"] == "context"]
        assert len(context_files) == 1
        ctx = json.loads(Path(context_files[0]["filePath"]).read_text())
        assert ctx["tokensUsed"] == 200
        assert ctx["model"] == "llama3.2:3b"
        assert ctx["expertId"] == "exp-123"

        # Verify prompt files
        prompt_files = [f for f in result["files"] if f["category"] == "prompt"]
        assert len(prompt_files) == 1

        system_files = [f for f in result["files"] if f["category"] == "system_prompt"]
        assert len(system_files) == 1

    def test_save_response_no_prompts_when_empty(self, temp_artifacts):
        artifacts = temp_artifacts
        result = artifacts.save_response(
            expert_id="exp-1",
            expert_name="Test",
            response="response",
        )
        categories = [f["category"] for f in result["files"]]
        assert "prompt" not in categories
        assert "system_prompt" not in categories

    def test_save_response_extracts_scripts(self, temp_artifacts):
        artifacts = temp_artifacts
        result = artifacts.save_response(
            expert_id="exp-1",
            expert_name="Coder Expert",
            response="```python\nprint('hello')\n```\n\n```bash\necho world\n```",
        )
        script_files = [f for f in result["files"] if f["category"] == "script"]
        assert len(script_files) == 2

    def test_date_based_directory_structure(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        result = artifacts.save_response(
            expert_id="exp-1",
            expert_name="My Expert",
            response="test",
        )
        artifact_dir = Path(result["artifactDir"])
        # Verify date directory is a parent
        date = result["date"]
        assert date in str(artifact_dir)
        assert "my-expert" in str(artifact_dir)

    def test_file_info_includes_metadata(self, temp_artifacts):
        artifacts = temp_artifacts
        result = artifacts.save_response(
            expert_id="exp-1",
            expert_name="Test Expert",
            response="test",
        )
        for f in result["files"]:
            assert "fileName" in f
            assert "filePath" in f
            assert "sizeBytes" in f
            assert "mimeType" in f
            assert "fileType" in f
            assert "date" in f
            assert "expertName" in f


class TestScriptExtraction:
    def test_extracts_python_script(self, temp_artifacts):
        artifacts = temp_artifacts
        scripts = artifacts.extract_and_save_scripts("Test", "```python\nprint('hello')\n```", "2026-03-22")
        assert len(scripts) == 1
        assert scripts[0].suffix == ".py"
        assert "print('hello')" in scripts[0].read_text()

    def test_extracts_multiple_languages(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```python\nx = 1\n```\n\n```bash\necho hi\n```\n\n```sql\nSELECT 1;\n```"
        scripts = artifacts.extract_and_save_scripts("Test", response, "2026-03-22")
        assert len(scripts) == 3
        exts = {s.suffix for s in scripts}
        assert ".py" in exts
        assert ".sh" in exts
        assert ".sql" in exts

    def test_no_scripts_in_plain_text(self, temp_artifacts):
        artifacts = temp_artifacts
        scripts = artifacts.extract_and_save_scripts("Test", "Just plain text.", "2026-03-22")
        assert len(scripts) == 0

    def test_scripts_in_scripts_subdir(self, temp_artifacts):
        artifacts = temp_artifacts
        scripts = artifacts.extract_and_save_scripts("Test", "```python\nx=1\n```", "2026-03-22")
        assert "scripts" in str(scripts[0].parent)

    def test_executable_bits(self, temp_artifacts):
        artifacts = temp_artifacts
        scripts = artifacts.extract_and_save_scripts("Test", "```bash\necho hi\n```", "2026-03-22")
        assert scripts[0].stat().st_mode & 0o100  # executable


class TestListArtifacts:
    def test_list_all_artifacts(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        artifacts.save_response(expert_id="e1", expert_name="Expert A", response="resp1")
        artifacts.save_response(expert_id="e2", expert_name="Expert B", response="resp2")
        all_artifacts = artifacts.list_all_artifacts()
        assert len(all_artifacts) >= 4  # at least 2 responses + 2 contexts

    def test_list_by_expert_name(self, temp_artifacts):
        artifacts = temp_artifacts
        artifacts.save_response(expert_id="e1", expert_name="Alpha Expert", response="resp1")
        artifacts.save_response(expert_id="e2", expert_name="Beta Expert", response="resp2")
        alpha_artifacts = artifacts.list_artifacts(expert_name="Alpha Expert")
        assert len(alpha_artifacts) >= 2
        assert all("alpha-expert" in a["fileName"] or "alpha" in a.get("expertName", "").lower() for a in alpha_artifacts)

    def test_list_by_date(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        # Manually create a dated directory
        date_dir = tmp_path / "2026-01-15" / "test-expert"
        date_dir.mkdir(parents=True)
        (date_dir / "test.md").write_text("content")

        result = artifacts.list_artifacts(date="2026-01-15")
        assert len(result) == 1

    def test_list_empty(self, temp_artifacts):
        artifacts = temp_artifacts
        result = artifacts.list_artifacts(expert_name="Nonexistent")
        assert result == []


class TestSaveArbitrary:
    def test_save_text_artifact(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_artifact("Test Expert", "output.txt", "content here", "2026-03-22")
        assert path.exists()
        assert path.read_text() == "content here"

    def test_save_binary_artifact(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_artifact("Test Expert", "data.bin", b"\x00\x01\x02", "2026-03-22")
        assert path.exists()
        assert path.read_bytes() == b"\x00\x01\x02"

    def test_save_nested_artifact(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_artifact("Test Expert", "subdir/nested.txt", "nested content", "2026-03-22")
        assert path.exists()
        assert "subdir" in str(path)


class TestCleanup:
    def test_cleanup_empty(self, temp_artifacts):
        artifacts = temp_artifacts
        result = artifacts.cleanup(max_age_days=30, max_total_mb=500)
        assert result["files_removed"] == 0
        assert result["bytes_freed"] == 0


class TestSlugify:
    def test_basic(self):
        assert ea._slugify("Research Analyst") == "research-analyst"

    def test_special_chars(self):
        assert ea._slugify("test@#$%^&*()") == "test"

    def test_empty(self):
        assert ea._slugify("") == "unnamed"

    def test_long_string(self):
        result = ea._slugify("a" * 200)
        assert len(result) <= 80
