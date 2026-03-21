"""Step artifacts — persists execution outputs, scripts, and context to disk."""

from __future__ import annotations

import asyncio
import json
import logging
import os
import re
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

logger = logging.getLogger("engine.step_artifacts")

STEPS_ROOT = Path(__file__).resolve().parents[3] / "steps" / "execution"


def _slugify(text: str) -> str:
    """Convert text to a filesystem-safe slug."""
    slug = re.sub(r"[^\w\s-]", "", text.lower().strip())
    return re.sub(r"[-\s]+", "-", slug)[:80] or "unnamed"


class StepArtifacts:
    """Manages execution artifacts on disk for each workflow step."""

    def __init__(self) -> None:
        STEPS_ROOT.mkdir(parents=True, exist_ok=True)

    def get_step_dir(self, workflow_name: str, step_name: str) -> Path:
        """Get or create the directory for a step's artifacts."""
        wf_slug = _slugify(workflow_name)
        step_slug = _slugify(step_name) if step_name else f"step-{int(time.time() * 1000)}"
        step_dir = STEPS_ROOT / wf_slug / step_slug
        step_dir.mkdir(parents=True, exist_ok=True)
        return step_dir

    def save_response(
        self,
        workflow_name: str,
        step_name: str,
        run_id: str,
        agent_id: str,
        response: str,
        *,
        prompt: str = "",
        system_prompt: str = "",
        model: str = "",
        tokens_used: int = 0,
        duration_ms: float = 0,
        metadata: dict[str, Any] | None = None,
    ) -> Path:
        """Save the full response and context for a step execution."""
        step_dir = self.get_step_dir(workflow_name, step_name)
        ts = datetime.now(UTC).strftime("%Y%m%d_%H%M%S")

        # Save response
        response_file = step_dir / f"response_{ts}.md"
        response_file.write_text(response, encoding="utf-8")

        # Save full context as JSON
        context = {
            "runId": run_id,
            "agentId": agent_id,
            "model": model,
            "tokensUsed": tokens_used,
            "durationMs": duration_ms,
            "timestamp": datetime.now(UTC).isoformat(),
            "promptLength": len(prompt),
            "responseLength": len(response),
            **(metadata or {}),
        }
        context_file = step_dir / f"context_{ts}.json"
        context_file.write_text(json.dumps(context, indent=2), encoding="utf-8")

        # Save prompts
        if prompt:
            (step_dir / f"prompt_{ts}.md").write_text(prompt, encoding="utf-8")
        if system_prompt:
            (step_dir / f"system_{ts}.md").write_text(system_prompt, encoding="utf-8")

        logger.info("Artifacts saved: %s/%s (%d bytes response)", workflow_name, step_name, len(response))
        return response_file

    def save_config(self, workflow_name: str, step_name: str, config: dict[str, Any]) -> Path:
        """Save step configuration."""
        step_dir = self.get_step_dir(workflow_name, step_name)
        config_file = step_dir / "config.json"
        config_file.write_text(json.dumps(config, indent=2), encoding="utf-8")
        return config_file

    def save_artifact(self, workflow_name: str, step_name: str, filename: str, content: str | bytes) -> Path:
        """Save an arbitrary artifact file."""
        step_dir = self.get_step_dir(workflow_name, step_name)
        artifact_path = step_dir / filename
        if isinstance(content, bytes):
            artifact_path.write_bytes(content)
        else:
            artifact_path.write_text(content, encoding="utf-8")
        return artifact_path

    def extract_and_save_scripts(self, workflow_name: str, step_name: str, response: str) -> list[Path]:
        """Extract code blocks from response, save as executable scripts."""
        scripts: list[Path] = []
        step_dir = self.get_step_dir(workflow_name, step_name)
        scripts_dir = step_dir / "scripts"
        scripts_dir.mkdir(exist_ok=True)

        # Match fenced code blocks with language hints
        pattern = r"```(\w+)?\n(.*?)```"
        matches = re.findall(pattern, response, re.DOTALL)

        for i, (lang, code) in enumerate(matches):
            lang = lang.lower() if lang else "txt"
            ext_map = {
                "python": ".py",
                "py": ".py",
                "bash": ".sh",
                "sh": ".sh",
                "shell": ".sh",
                "zsh": ".sh",
                "javascript": ".js",
                "js": ".js",
                "node": ".js",
                "typescript": ".ts",
                "ts": ".ts",
                "go": ".go",
                "golang": ".go",
                "sql": ".sql",
                "json": ".json",
                "yaml": ".yaml",
                "yml": ".yaml",
                "toml": ".toml",
                "xml": ".xml",
                "html": ".html",
                "css": ".css",
            }
            ext = ext_map.get(lang, ".txt")
            script_file = scripts_dir / f"script_{i + 1}{ext}"
            script_file.write_text(code.strip(), encoding="utf-8")

            # Make shell/python scripts executable
            if ext in (".sh", ".py"):
                script_file.chmod(0o755)

            scripts.append(script_file)
            logger.info("Extracted script: %s (%s, %d bytes)", script_file.name, lang, len(code))

        return scripts

    async def execute_script(
        self,
        script_path: Path,
        *,
        timeout: int = 120,
        env: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        """Execute a script and return the result. Supports .py, .sh, .js, .ts."""
        if not script_path.exists():
            return {"error": f"Script not found: {script_path}", "exitCode": -1, "stdout": "", "stderr": ""}

        ext = script_path.suffix.lower()
        interpreters: dict[str, list[str]] = {
            ".py": ["python3"],
            ".sh": ["bash"],
            ".js": ["node"],
            ".ts": ["npx", "tsx"],
        }

        cmd = interpreters.get(ext)
        if not cmd:
            return {"error": f"Unsupported script type: {ext}", "exitCode": -1, "stdout": "", "stderr": ""}

        cmd = [*cmd, str(script_path)]
        run_env = {**os.environ, **(env or {})}

        start = time.monotonic()
        try:
            proc = await asyncio.create_subprocess_exec(
                *cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=str(script_path.parent),
                env=run_env,
            )
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=timeout)
            duration_ms = int((time.monotonic() - start) * 1000)

            result: dict[str, Any] = {
                "exitCode": proc.returncode,
                "stdout": stdout.decode("utf-8", errors="replace"),
                "stderr": stderr.decode("utf-8", errors="replace"),
                "durationMs": duration_ms,
                "script": script_path.name,
            }

            # Save execution result
            result_file = script_path.parent / f"{script_path.stem}_result.json"
            result_file.write_text(json.dumps(result, indent=2), encoding="utf-8")

            logger.info(
                "Script executed: %s (exit=%d, %dms)",
                script_path.name,
                proc.returncode,
                duration_ms,
            )
            return result

        except TimeoutError:
            duration_ms = int((time.monotonic() - start) * 1000)
            return {
                "error": f"Script timed out after {timeout}s",
                "exitCode": -1,
                "stdout": "",
                "stderr": "",
                "durationMs": duration_ms,
                "script": script_path.name,
            }
        except Exception as e:
            return {
                "error": str(e),
                "exitCode": -1,
                "stdout": "",
                "stderr": "",
                "script": script_path.name,
            }

    def save_failure_log(
        self,
        workflow_name: str,
        step_name: str,
        run_id: str,
        error: str,
        *,
        agent_id: str = "",
        phase: str = "",
        metadata: dict[str, Any] | None = None,
    ) -> Path:
        """Save a detailed failure log for a step."""
        step_dir = self.get_step_dir(workflow_name, step_name)
        ts = datetime.now(UTC).strftime("%Y%m%d_%H%M%S")

        failure = {
            "runId": run_id,
            "agentId": agent_id,
            "phase": phase,
            "error": error,
            "timestamp": datetime.now(UTC).isoformat(),
            **(metadata or {}),
        }

        failure_file = step_dir / f"failure_{ts}.json"
        failure_file.write_text(json.dumps(failure, indent=2), encoding="utf-8")
        return failure_file

    def list_artifacts(self, workflow_name: str, step_name: str) -> list[dict[str, Any]]:
        """List all artifacts for a step."""
        step_dir = self.get_step_dir(workflow_name, step_name)
        if not step_dir.exists():
            return []

        artifacts: list[dict[str, Any]] = []
        for f in sorted(step_dir.rglob("*")):
            if f.is_file():
                rel = f.relative_to(step_dir)
                artifacts.append(
                    {
                        "name": str(rel),
                        "size": f.stat().st_size,
                        "modified": datetime.fromtimestamp(f.stat().st_mtime, tz=UTC).isoformat(),
                        "type": f.suffix,
                    }
                )
        return artifacts


step_artifacts = StepArtifacts()
