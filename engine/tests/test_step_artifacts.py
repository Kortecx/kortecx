"""Tests for step artifacts — disk persistence and script extraction."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

import engine.services.step_artifacts as sa


@pytest.fixture
def temp_artifacts(tmp_path, monkeypatch):
    monkeypatch.setattr(sa, "STEPS_ROOT", tmp_path)
    return sa.StepArtifacts()


class TestStepArtifacts:
    def test_get_step_dir_creates_directory(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        d = artifacts.get_step_dir("My Workflow", "Step One")
        assert d.exists()
        assert "my-workflow" in str(d)
        assert "step-one" in str(d)

    def test_get_step_dir_idempotent(self, temp_artifacts):
        artifacts = temp_artifacts
        d1 = artifacts.get_step_dir("wf", "step")
        d2 = artifacts.get_step_dir("wf", "step")
        assert d1 == d2

    def test_save_response(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_response(
            "test-wf", "step-1", "run-123", "agent-1",
            "Hello world response",
            prompt="user prompt", system_prompt="system prompt",
            model="llama3.2:3b", tokens_used=100, duration_ms=500,
        )
        assert path.exists()
        assert "Hello world" in path.read_text()
        # Check context file was created
        step_dir = path.parent
        context_files = list(step_dir.glob("context_*.json"))
        assert len(context_files) == 1
        ctx = json.loads(context_files[0].read_text())
        assert ctx["tokensUsed"] == 100

    def test_save_response_creates_prompt_files(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_response(
            "wf", "step", "r1", "a1", "response",
            prompt="user prompt", system_prompt="sys prompt",
        )
        step_dir = path.parent
        prompt_files = list(step_dir.glob("prompt_*.md"))
        system_files = list(step_dir.glob("system_*.md"))
        assert len(prompt_files) == 1
        assert len(system_files) == 1

    def test_save_response_no_prompt_files_when_empty(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_response("wf", "step", "r1", "a1", "response")
        step_dir = path.parent
        prompt_files = list(step_dir.glob("prompt_*.md"))
        system_files = list(step_dir.glob("system_*.md"))
        assert len(prompt_files) == 0
        assert len(system_files) == 0

    def test_save_config(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_config("wf", "step", {"key": "value"})
        assert path.exists()
        data = json.loads(path.read_text())
        assert data["key"] == "value"

    def test_save_artifact(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_artifact("wf", "step", "output.txt", "content here")
        assert path.exists()
        assert path.read_text() == "content here"

    def test_save_artifact_bytes(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_artifact("wf", "step", "data.bin", b"\x00\x01\x02")
        assert path.exists()
        assert path.read_bytes() == b"\x00\x01\x02"

    def test_save_failure_log(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_failure_log("wf", "step", "run-1", "Something went wrong", agent_id="ag-1")
        assert path.exists()
        data = json.loads(path.read_text())
        assert data["error"] == "Something went wrong"
        assert data["agentId"] == "ag-1"

    def test_save_failure_log_with_phase_and_metadata(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_failure_log(
            "wf", "step", "run-1", "Error",
            agent_id="ag-1", phase="execute",
            metadata={"retry": 2},
        )
        data = json.loads(path.read_text())
        assert data["phase"] == "execute"
        assert data["retry"] == 2

    def test_list_artifacts(self, temp_artifacts):
        artifacts = temp_artifacts
        artifacts.save_response("wf", "step", "r1", "a1", "response1")
        artifacts.save_config("wf", "step", {"x": 1})
        result = artifacts.list_artifacts("wf", "step")
        assert len(result) >= 3  # response + context + config

    def test_list_artifacts_empty(self, temp_artifacts):
        artifacts = temp_artifacts
        # list_artifacts calls get_step_dir which creates the dir, so it will
        # return an empty list when no files are in it
        result = artifacts.list_artifacts("nonexistent", "step")
        assert result == []

    def test_list_artifacts_includes_nested(self, temp_artifacts):
        artifacts = temp_artifacts
        # Create a response which also extracts scripts into a subfolder
        response = "```python\nprint('hi')\n```"
        artifacts.save_response("wf", "step", "r1", "a1", "resp")
        artifacts.extract_and_save_scripts("wf", "step", response)
        result = artifacts.list_artifacts("wf", "step")
        names = [a["name"] for a in result]
        assert any("script" in n for n in names)

    def test_save_response_context_metadata(self, temp_artifacts):
        artifacts = temp_artifacts
        path = artifacts.save_response(
            "wf", "step", "run-1", "agent-1", "response text",
            model="gpt-4", tokens_used=500, duration_ms=1200,
        )
        step_dir = path.parent
        context_files = list(step_dir.glob("context_*.json"))
        ctx = json.loads(context_files[0].read_text())
        assert ctx["model"] == "gpt-4"
        assert ctx["durationMs"] == 1200
        assert ctx["responseLength"] == len("response text")


class TestScriptExtraction:
    def test_extracts_python_script(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "Here is code:\n```python\nprint('hello')\n```\nDone."
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert len(scripts) == 1
        assert scripts[0].suffix == ".py"
        assert "print('hello')" in scripts[0].read_text()

    def test_extracts_multiple_scripts(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```python\nx = 1\n```\n\n```bash\necho hi\n```\n\n```sql\nSELECT 1;\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert len(scripts) == 3
        exts = {s.suffix for s in scripts}
        assert ".py" in exts
        assert ".sh" in exts
        assert ".sql" in exts

    def test_no_scripts_in_plain_text(self, temp_artifacts):
        artifacts = temp_artifacts
        scripts = artifacts.extract_and_save_scripts("wf", "step", "Just plain text, no code blocks.")
        assert len(scripts) == 0

    def test_makes_scripts_executable(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```bash\necho hello\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert scripts[0].stat().st_mode & 0o100  # executable bit

    def test_python_script_executable(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```python\nprint(1)\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert scripts[0].stat().st_mode & 0o100

    def test_sql_script_not_executable(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```sql\nSELECT 1;\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert not (scripts[0].stat().st_mode & 0o100)

    def test_scripts_in_scripts_subdir(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        response = "```python\nx=1\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert "scripts" in str(scripts[0].parent)

    def test_extracts_js_and_ts(self, temp_artifacts):
        artifacts = temp_artifacts
        response = "```javascript\nconsole.log(1)\n```\n```typescript\nconst x: number = 1;\n```"
        scripts = artifacts.extract_and_save_scripts("wf", "step", response)
        assert len(scripts) == 2
        exts = {s.suffix for s in scripts}
        assert ".js" in exts
        assert ".ts" in exts

    @pytest.mark.asyncio
    async def test_execute_python_script(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        script = tmp_path / "test.py"
        script.write_text("print('result')")
        result = await artifacts.execute_script(script, timeout=10)
        assert result["exitCode"] == 0
        assert "result" in result["stdout"]

    @pytest.mark.asyncio
    async def test_execute_missing_script(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        result = await artifacts.execute_script(tmp_path / "nonexistent.py")
        assert result["exitCode"] == -1
        assert "not found" in result["error"].lower()

    @pytest.mark.asyncio
    async def test_execute_unsupported_type(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        script = tmp_path / "test.rb"
        script.write_text("puts 'hello'")
        result = await artifacts.execute_script(script)
        assert result["exitCode"] == -1
        assert "unsupported" in result["error"].lower()

    @pytest.mark.asyncio
    async def test_execute_script_saves_result(self, temp_artifacts, tmp_path):
        artifacts = temp_artifacts
        script = tmp_path / "test.py"
        script.write_text("print('ok')")
        await artifacts.execute_script(script, timeout=10)
        result_file = tmp_path / "test_result.json"
        assert result_file.exists()
        data = json.loads(result_file.read_text())
        assert data["exitCode"] == 0


class TestSlugify:
    def test_basic(self):
        assert sa._slugify("My Workflow") == "my-workflow"

    def test_special_chars(self):
        assert sa._slugify("test@#$%^&*()") == "test"

    def test_empty(self):
        result = sa._slugify("")
        assert result == "unnamed"

    def test_long_string(self):
        result = sa._slugify("a" * 200)
        assert len(result) <= 80

    def test_multiple_spaces(self):
        assert sa._slugify("hello   world") == "hello-world"

    def test_underscores_preserved(self):
        result = sa._slugify("hello_world")
        assert "hello" in result and "world" in result

    def test_mixed_case(self):
        assert sa._slugify("Hello World") == "hello-world"
