"""Workflow artifacts — persists workflow run outputs, step responses, and scripts to disk.

Storage structure:
  outputs/workflows/{workflow_slug}/{run_id}/
    summary.md          — final workflow output
    context.json        — run metadata (steps, tokens, duration, status)
    goal.md             — original goal/input
    steps/
      {step_number}_{step_slug}/
        response.md     — step model response
        prompt.md       — user prompt sent
        system.md       — system prompt used
        context.json    — step metadata (model, tokens, duration)
        scripts/        — extracted code blocks
"""

from __future__ import annotations

import json
import logging
import mimetypes
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

logger = logging.getLogger("engine.workflow_artifacts")

OUTPUTS_ROOT = Path(__file__).resolve().parents[3] / "outputs" / "workflows"

_EXT_MAP: dict[str, str] = {
    "python": ".py", "py": ".py", "bash": ".sh", "sh": ".sh", "shell": ".sh",
    "zsh": ".sh", "javascript": ".js", "js": ".js", "node": ".js",
    "typescript": ".ts", "ts": ".ts", "go": ".go", "golang": ".go",
    "sql": ".sql", "json": ".json", "yaml": ".yaml", "yml": ".yaml",
    "toml": ".toml", "xml": ".xml", "html": ".html", "css": ".css",
}


def _slugify(text: str) -> str:
    slug = re.sub(r"[^\w\s-]", "", text.lower().strip())
    return re.sub(r"[-\s]+", "-", slug)[:80] or "unnamed"


def _detect_mime(path: Path) -> str:
    mime, _ = mimetypes.guess_type(str(path))
    return mime or "application/octet-stream"


class WorkflowArtifacts:
    """Manages workflow execution artifacts on disk, organized per-run."""

    def __init__(self) -> None:
        OUTPUTS_ROOT.mkdir(parents=True, exist_ok=True)

    def _run_dir(self, workflow_name: str, run_id: str) -> Path:
        wf_slug = _slugify(workflow_name)
        run_dir = OUTPUTS_ROOT / wf_slug / run_id
        run_dir.mkdir(parents=True, exist_ok=True)
        return run_dir

    def _step_dir(self, workflow_name: str, run_id: str, step_number: int, step_name: str) -> Path:
        step_slug = f"{step_number:02d}_{_slugify(step_name)}"
        step_path = self._run_dir(workflow_name, run_id) / "steps" / step_slug
        step_path.mkdir(parents=True, exist_ok=True)
        return step_path

    # ── Save methods ─────────────────────────────────────────────────────────

    def save_run_summary(
        self,
        workflow_name: str,
        workflow_id: str,
        run_id: str,
        *,
        output: str = "",
        goal: str = "",
        status: str = "completed",
        total_tokens: int = 0,
        duration_sec: int = 0,
        steps_completed: int = 0,
        total_steps: int = 0,
        expert_chain: list[str] | None = None,
        error_message: str = "",
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Save the final run summary and context."""
        run_dir = self._run_dir(workflow_name, run_id)
        saved: list[dict[str, Any]] = []

        # summary.md — final output
        if output:
            frontmatter = (
                f"---\nworkflow: {workflow_name}\nworkflow_id: {workflow_id}\n"
                f"run_id: {run_id}\nstatus: {status}\n"
                f"tokens: {total_tokens}\nduration_sec: {duration_sec}\n"
                f"timestamp: {datetime.now(UTC).isoformat()}\n---\n\n"
            )
            summary_file = run_dir / "summary.md"
            summary_file.write_text(frontmatter + output, encoding="utf-8")
            saved.append(self._file_info(summary_file, "summary", run_id))

        # context.json — run metadata
        ctx: dict[str, Any] = {
            "workflowId": workflow_id,
            "workflowName": workflow_name,
            "runId": run_id,
            "status": status,
            "totalTokens": total_tokens,
            "durationSec": duration_sec,
            "stepsCompleted": steps_completed,
            "totalSteps": total_steps,
            "expertChain": expert_chain or [],
            "errorMessage": error_message,
            "timestamp": datetime.now(UTC).isoformat(),
            **(metadata or {}),
        }
        ctx_file = run_dir / "context.json"
        ctx_file.write_text(json.dumps(ctx, indent=2), encoding="utf-8")
        saved.append(self._file_info(ctx_file, "context", run_id))

        # goal.md — original input
        if goal:
            goal_file = run_dir / "goal.md"
            goal_file.write_text(goal, encoding="utf-8")
            saved.append(self._file_info(goal_file, "goal", run_id))

        logger.info("Workflow run summary saved: %s/%s (%d files)", _slugify(workflow_name), run_id, len(saved))
        return {"runId": run_id, "artifactDir": str(run_dir), "files": saved}

    def save_step_output(
        self,
        workflow_name: str,
        run_id: str,
        step_number: int,
        step_name: str,
        *,
        response: str = "",
        prompt: str = "",
        system_prompt: str = "",
        model: str = "",
        engine: str = "",
        tokens_used: int = 0,
        duration_ms: float = 0,
        status: str = "completed",
        error_message: str = "",
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Save outputs for a single workflow step."""
        step_dir = self._step_dir(workflow_name, run_id, step_number, step_name)
        saved: list[dict[str, Any]] = []

        # response.md
        if response:
            frontmatter = (
                f"---\nstep: {step_name}\nstep_number: {step_number}\n"
                f"model: {model}\nengine: {engine}\n"
                f"tokens: {tokens_used}\nduration_ms: {duration_ms}\n"
                f"status: {status}\ntimestamp: {datetime.now(UTC).isoformat()}\n---\n\n"
            )
            resp_file = step_dir / "response.md"
            resp_file.write_text(frontmatter + response, encoding="utf-8")
            saved.append(self._file_info(resp_file, "response", run_id))

            # Extract scripts
            for sf in self._extract_scripts(step_dir, response):
                saved.append(self._file_info(sf, "script", run_id))

        # prompt.md
        if prompt:
            pf = step_dir / "prompt.md"
            pf.write_text(prompt, encoding="utf-8")
            saved.append(self._file_info(pf, "prompt", run_id))

        # system.md
        if system_prompt:
            sf = step_dir / "system.md"
            sf.write_text(system_prompt, encoding="utf-8")
            saved.append(self._file_info(sf, "system_prompt", run_id))

        # context.json
        ctx: dict[str, Any] = {
            "stepName": step_name,
            "stepNumber": step_number,
            "model": model,
            "engine": engine,
            "tokensUsed": tokens_used,
            "durationMs": duration_ms,
            "status": status,
            "errorMessage": error_message,
            "timestamp": datetime.now(UTC).isoformat(),
            **(metadata or {}),
        }
        cf = step_dir / "context.json"
        cf.write_text(json.dumps(ctx, indent=2), encoding="utf-8")
        saved.append(self._file_info(cf, "context", run_id))

        logger.info("Step output saved: %s/%s/step_%02d_%s (%d files)",
                     _slugify(workflow_name), run_id, step_number, _slugify(step_name), len(saved))
        return {"stepName": step_name, "stepNumber": step_number, "files": saved}

    # ── Listing ──────────────────────────────────────────────────────────────

    def list_runs(self, workflow_name: str) -> list[dict[str, Any]]:
        """List all run folders for a workflow, newest first."""
        wf_slug = _slugify(workflow_name)
        wf_dir = OUTPUTS_ROOT / wf_slug
        if not wf_dir.exists():
            return []

        runs: list[dict[str, Any]] = []
        for run_dir in sorted(wf_dir.iterdir(), reverse=True):
            if not run_dir.is_dir() or run_dir.name.startswith(("_", ".")):
                continue
            files = self._scan_dir(run_dir, run_dir.name)
            total_size = sum(f["sizeBytes"] for f in files)
            # Try to read context.json for metadata
            ctx_path = run_dir / "context.json"
            ctx_data: dict[str, Any] = {}
            if ctx_path.exists():
                try:
                    ctx_data = json.loads(ctx_path.read_text(encoding="utf-8"))
                except Exception:
                    pass
            runs.append({
                "runId": run_dir.name,
                "workflowSlug": wf_slug,
                "workflowName": workflow_name,
                "artifactDir": str(run_dir),
                "fileCount": len(files),
                "totalSize": total_size,
                "status": ctx_data.get("status", "unknown"),
                "totalTokens": ctx_data.get("totalTokens", 0),
                "durationSec": ctx_data.get("durationSec", 0),
                "timestamp": ctx_data.get("timestamp", ""),
                "files": files,
            })
        return runs

    def get_file_content(self, workflow_name: str, run_id: str, filename: str) -> str | None:
        """Read content of a specific file in a run folder."""
        wf_slug = _slugify(workflow_name)
        file_path = OUTPUTS_ROOT / wf_slug / run_id / filename
        if file_path.exists() and file_path.is_file():
            return file_path.read_text(encoding="utf-8")
        return None

    # ── Internal ─────────────────────────────────────────────────────────────

    def _extract_scripts(self, parent_dir: Path, response: str) -> list[Path]:
        pattern = r"```(\w+)?\n(.*?)```"
        matches = re.findall(pattern, response, re.DOTALL)
        if not matches:
            return []
        scripts_dir = parent_dir / "scripts"
        scripts_dir.mkdir(exist_ok=True)
        scripts: list[Path] = []
        for i, (lang, code) in enumerate(matches):
            lang = lang.lower() if lang else "txt"
            ext = _EXT_MAP.get(lang, ".txt")
            sf = scripts_dir / f"script_{i + 1}{ext}"
            sf.write_text(code.strip(), encoding="utf-8")
            if ext in (".sh", ".py"):
                sf.chmod(0o755)
            scripts.append(sf)
        return scripts

    def _file_info(self, path: Path, category: str, run_id: str) -> dict[str, Any]:
        stat = path.stat()
        return {
            "fileName": path.name,
            "filePath": str(path),
            "sizeBytes": stat.st_size,
            "mimeType": _detect_mime(path),
            "category": category,
            "runId": run_id,
            "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
        }

    def _scan_dir(self, directory: Path, run_id: str) -> list[dict[str, Any]]:
        artifacts: list[dict[str, Any]] = []
        for f in sorted(directory.rglob("*")):
            if f.is_file():
                try:
                    stat = f.stat()
                    rel = f.relative_to(directory)
                    artifacts.append({
                        "fileName": str(rel),
                        "filePath": str(f),
                        "sizeBytes": stat.st_size,
                        "mimeType": _detect_mime(f),
                        "runId": run_id,
                        "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
                    })
                except OSError:
                    continue
        return artifacts


workflow_artifacts = WorkflowArtifacts()
