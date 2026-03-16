"""Workflow interaction logger — persists all user interactions as .log and .md files.

Log structure:
  logs/
    workflows/
      <workflow-id>/
        interactions.log       — timestamped interaction events
        config.md              — workflow configuration snapshot
        goal.md                — task goal content
        context/
          <filename>           — uploaded input files
        runs/
          <run-id>/
            execution.log      — agent execution events
            memory.md          — shared memory snapshots
            output.md          — final output
    sessions/
      <session-id>.log         — session-level interaction log
    metrics/
      <workflow-id>.log        — metrics configuration and tracking events
    permissions/
      <workflow-id>.md         — permission changes
    tags/
      <workflow-id>.md         — tag changes
"""

from __future__ import annotations

import json
import logging
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from engine.config import settings

logger = logging.getLogger("engine.workflow_logger")

LOG_BASE = Path(settings.upload_dir).parent / "logs"


def _ts() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%S.%fZ")


def _ts_short() -> str:
    return datetime.now(UTC).strftime("%Y-%m-%d %H:%M:%S")


def _ensure(path: Path) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    return path


def _append_log(path: Path, entry: str) -> None:
    _ensure(path)
    with open(path, "a", encoding="utf-8") as f:
        f.write(entry)
        if not entry.endswith("\n"):
            f.write("\n")


def _write_md(path: Path, content: str) -> None:
    _ensure(path)
    path.write_text(content, encoding="utf-8")


class WorkflowLogger:
    """Persists workflow interactions, configs, and execution events to disk."""

    def __init__(self, base: Path | None = None) -> None:
        self.base = base or LOG_BASE
        self.base.mkdir(parents=True, exist_ok=True)

    # ── Paths ────────────────────────────────────────────

    def _wf_dir(self, workflow_id: str) -> Path:
        return self.base / "workflows" / workflow_id

    def _run_dir(self, workflow_id: str, run_id: str) -> Path:
        return self._wf_dir(workflow_id) / "runs" / run_id

    def _session_path(self, session_id: str) -> Path:
        return self.base / "sessions" / f"{session_id}.log"

    def _metrics_path(self, workflow_id: str) -> Path:
        return self.base / "metrics" / f"{workflow_id}.log"

    def _permissions_path(self, workflow_id: str) -> Path:
        return self.base / "permissions" / f"{workflow_id}.md"

    def _tags_path(self, workflow_id: str) -> Path:
        return self.base / "tags" / f"{workflow_id}.md"

    # ── Session Logging ──────────────────────────────────

    def log_session_event(
        self,
        session_id: str,
        event_type: str,
        data: dict[str, Any] | None = None,
    ) -> None:
        """Log a session-level event."""
        entry = f"[{_ts_short()}] {event_type}"
        if data:
            entry += f" | {json.dumps(data, default=str)}"
        _append_log(self._session_path(session_id), entry)

    # ── Workflow Interaction Logging ─────────────────────

    def log_interaction(
        self,
        workflow_id: str,
        action: str,
        details: dict[str, Any] | None = None,
    ) -> None:
        """Log a user interaction with the workflow builder."""
        path = self._wf_dir(workflow_id) / "interactions.log"
        entry = f"[{_ts_short()}] {action}"
        if details:
            entry += f" | {json.dumps(details, default=str)}"
        _append_log(path, entry)

    # ── Goal Logging ─────────────────────────────────────

    def save_goal(
        self,
        workflow_id: str,
        goal_content: str,
        source: str = "text",
    ) -> None:
        """Persist the task goal as a .md file."""
        path = self._wf_dir(workflow_id) / "goal.md"
        header = f"---\nworkflow: {workflow_id}\nsource: {source}\nsaved_at: {_ts()}\n---\n\n"
        _write_md(path, header + goal_content)
        self.log_interaction(workflow_id, "goal.saved", {"source": source, "length": len(goal_content)})

    # ── Config Logging ───────────────────────────────────

    def save_config(
        self,
        workflow_id: str,
        config: dict[str, Any],
    ) -> None:
        """Persist the workflow configuration as a .md snapshot."""
        path = self._wf_dir(workflow_id) / "config.md"
        lines = [
            "---",
            f"workflow: {workflow_id}",
            f"saved_at: {_ts()}",
            "---",
            "",
            "# Workflow Configuration",
            "",
        ]
        for section, values in config.items():
            lines.append(f"## {section}")
            lines.append("")
            if isinstance(values, dict):
                for k, v in values.items():
                    lines.append(f"- **{k}**: {v}")
            elif isinstance(values, list):
                for item in values:
                    lines.append(f"- {item}")
            else:
                lines.append(f"{values}")
            lines.append("")

        _write_md(path, "\n".join(lines))
        self.log_interaction(workflow_id, "config.saved", {"sections": list(config.keys())})

    # ── Metrics Logging ──────────────────────────────────

    def log_metrics_config(
        self,
        workflow_id: str,
        metrics_config: dict[str, Any],
    ) -> None:
        """Log metrics configuration changes (MLflow, logging, monitoring)."""
        entry = f"[{_ts_short()}] metrics.config.updated | {json.dumps(metrics_config, default=str)}"
        _append_log(self._metrics_path(workflow_id), entry)
        self.log_interaction(workflow_id, "metrics.config.updated", metrics_config)

    # ── Tags Logging ─────────────────────────────────────

    def save_tags(
        self,
        workflow_id: str,
        tags: list[str],
    ) -> None:
        """Persist tag state as a .md file."""
        lines = [
            "---",
            f"workflow: {workflow_id}",
            f"updated_at: {_ts()}",
            "---",
            "",
            "# Workflow Tags",
            "",
        ]
        for tag in tags:
            lines.append(f"- `{tag}`")
        _write_md(self._tags_path(workflow_id), "\n".join(lines))
        self.log_interaction(workflow_id, "tags.updated", {"tags": tags})

    # ── Permissions Logging ──────────────────────────────

    def save_permissions(
        self,
        workflow_id: str,
        permissions: dict[str, Any],
    ) -> None:
        """Persist permission settings as a .md file."""
        lines = [
            "---",
            f"workflow: {workflow_id}",
            f"updated_at: {_ts()}",
            "---",
            "",
            "# Workflow Permissions",
            "",
        ]
        for key, value in permissions.items():
            lines.append(f"- **{key}**: {value}")
        _write_md(self._permissions_path(workflow_id), "\n".join(lines))
        self.log_interaction(workflow_id, "permissions.updated", permissions)

    # ── Step Logging ─────────────────────────────────────

    def log_step_change(
        self,
        workflow_id: str,
        action: str,
        step_data: dict[str, Any],
    ) -> None:
        """Log step add/remove/update events."""
        self.log_interaction(workflow_id, f"step.{action}", step_data)

    # ── Run Execution Logging ────────────────────────────

    def log_run_event(
        self,
        workflow_id: str,
        run_id: str,
        event: str,
        data: dict[str, Any] | None = None,
    ) -> None:
        """Log a run execution event."""
        path = self._run_dir(workflow_id, run_id) / "execution.log"
        entry = f"[{_ts_short()}] {event}"
        if data:
            entry += f" | {json.dumps(data, default=str)}"
        _append_log(path, entry)

    def save_run_memory(
        self,
        workflow_id: str,
        run_id: str,
        memory: dict[str, Any],
    ) -> None:
        """Persist shared memory snapshot."""
        path = self._run_dir(workflow_id, run_id) / "memory.md"
        lines = [
            "---",
            f"workflow: {workflow_id}",
            f"run: {run_id}",
            f"snapshot_at: {_ts()}",
            "---",
            "",
            "# Shared Memory Snapshot",
            "",
        ]
        for agent_id, mem in memory.get("entries", {}).items():
            lines.append(f"## Agent: {agent_id}")
            lines.append(f"```json\n{mem}\n```")
            lines.append("")
        if memory.get("globals"):
            lines.append("## Global Memory")
            for k, v in memory["globals"].items():
                lines.append(f"### {k}")
                lines.append(v[:500])
                lines.append("")
        _write_md(path, "\n".join(lines))

    def save_run_output(
        self,
        workflow_id: str,
        run_id: str,
        output: str,
    ) -> None:
        """Persist final workflow output."""
        path = self._run_dir(workflow_id, run_id) / "output.md"
        header = f"---\nworkflow: {workflow_id}\nrun: {run_id}\ncompleted_at: {_ts()}\n---\n\n# Workflow Output\n\n"
        _write_md(path, header + output)

    # ── Context File Logging ─────────────────────────────

    def save_context_file(
        self,
        workflow_id: str,
        filename: str,
        content: bytes | str,
    ) -> None:
        """Persist an uploaded context file."""
        path = self._wf_dir(workflow_id) / "context" / filename
        _ensure(path)
        if isinstance(content, str):
            path.write_text(content, encoding="utf-8")
        else:
            path.write_bytes(content)
        self.log_interaction(workflow_id, "context.file.saved", {"filename": filename, "size": len(content)})

    # ── Retrieval ────────────────────────────────────────

    def get_interaction_log(self, workflow_id: str) -> str:
        """Read the interaction log for a workflow."""
        path = self._wf_dir(workflow_id) / "interactions.log"
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def get_run_log(self, workflow_id: str, run_id: str) -> str:
        """Read execution log for a run."""
        path = self._run_dir(workflow_id, run_id) / "execution.log"
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def get_metrics_log(self, workflow_id: str) -> str:
        """Read metrics log for a workflow."""
        path = self._metrics_path(workflow_id)
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def get_session_log(self, session_id: str) -> str:
        """Read a session log."""
        path = self._session_path(session_id)
        return path.read_text(encoding="utf-8") if path.exists() else ""

    def list_workflow_logs(self, workflow_id: str) -> dict[str, Any]:
        """List all log files for a workflow."""
        wf_dir = self._wf_dir(workflow_id)
        if not wf_dir.exists():
            return {"files": []}
        files = []
        for p in sorted(wf_dir.rglob("*")):
            if p.is_file():
                files.append(
                    {
                        "path": str(p.relative_to(self.base)),
                        "size": p.stat().st_size,
                        "modified": datetime.fromtimestamp(p.stat().st_mtime, tz=UTC).isoformat(),
                        "type": "log" if p.suffix == ".log" else "md" if p.suffix == ".md" else "file",
                    }
                )
        return {"workflowId": workflow_id, "files": files}


workflow_logger = WorkflowLogger()
