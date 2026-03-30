"""Expert artifacts — persists expert run outputs, scripts, and context to disk.

Storage structure:
  outputs/agents/{agent_slug}/{YYYYMMDD_HHMMSS}/
    response.md      — full model response with YAML frontmatter
    context.json     — run metadata (model, tokens, duration, etc.)
    prompt.md        — user prompt sent (if any)
    system.md        — system prompt used (if any)
    scripts/         — code blocks extracted from response
      script_1.py
      script_2.sh
"""

from __future__ import annotations

import json
import logging
import mimetypes
import os
import re
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

logger = logging.getLogger("engine.expert_artifacts")

OUTPUTS_ROOT = Path(__file__).resolve().parents[3] / "outputs" / "agents"

# Extension to language mapping for script extraction
_EXT_MAP: dict[str, str] = {
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


def _slugify(text: str) -> str:
    """Convert text to a filesystem-safe slug."""
    slug = re.sub(r"[^\w\s-]", "", text.lower().strip())
    return re.sub(r"[-\s]+", "-", slug)[:80] or "unnamed"


def _detect_mime(path: Path) -> str:
    """Detect MIME type from file extension."""
    mime, _ = mimetypes.guess_type(str(path))
    return mime or "application/octet-stream"


def _detect_file_type(ext: str) -> str:
    """Categorize file by extension."""
    ext = ext.lower()
    if ext in (".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".bmp"):
        return "image"
    if ext in (".mp4", ".webm", ".avi", ".mov", ".mkv"):
        return "video"
    if ext in (".mp3", ".wav", ".ogg", ".flac", ".m4a"):
        return "audio"
    if ext in (".pdf", ".doc", ".docx", ".txt", ".md", ".rtf"):
        return "document"
    if ext in (".csv", ".jsonl", ".parquet", ".tsv"):
        return "dataset"
    return "file"


class ExpertArtifacts:
    """Manages expert execution artifacts on disk.

    Structure: outputs/agents/{agent_slug}/{YYYYMMDD_HHMMSS}/
    Each run gets its own timestamped folder so artifacts never mix.
    """

    def __init__(self) -> None:
        OUTPUTS_ROOT.mkdir(parents=True, exist_ok=True)

    def _make_run_dir(self, expert_name: str, run_ts: str) -> Path:
        """Create and return the directory for a single run."""
        expert_slug = _slugify(expert_name)
        run_dir = OUTPUTS_ROOT / expert_slug / run_ts
        run_dir.mkdir(parents=True, exist_ok=True)
        return run_dir

    def save_response(
        self,
        expert_id: str,
        expert_name: str,
        response: str,
        *,
        prompt: str = "",
        system_prompt: str = "",
        model: str = "",
        engine: str = "",
        tokens_used: int = 0,
        duration_ms: float = 0,
        tags: list[str] | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Save full expert run output to disk. Returns dict of saved file paths with sizes."""
        run_ts = datetime.now(UTC).strftime("%Y%m%d_%H%M%S")
        run_id = f"run-{run_ts}-{int(time.time() * 1000) % 100000}"
        expert_slug = _slugify(expert_name)
        run_dir = self._make_run_dir(expert_name, run_ts)

        saved_files: list[dict[str, Any]] = []

        # Save response with YAML frontmatter
        frontmatter = (
            f"---\nexpert: {expert_name}\nexpert_id: {expert_id}\n"
            f"model: {model}\nengine: {engine}\n"
            f"tokens: {tokens_used}\nduration_ms: {duration_ms}\n"
            f"timestamp: {datetime.now(UTC).isoformat()}\n"
            f"run_id: {run_id}\n"
            f"tags: [{', '.join(tags or [])}]\n---\n\n"
        )
        response_file = run_dir / "response.md"
        response_file.write_text(frontmatter + response, encoding="utf-8")
        saved_files.append(self._file_info(response_file, "response", expert_slug, run_ts))

        # Save context JSON
        context = {
            "expertId": expert_id,
            "expertName": expert_name,
            "runId": run_id,
            "model": model,
            "engine": engine,
            "tokensUsed": tokens_used,
            "durationMs": duration_ms,
            "timestamp": datetime.now(UTC).isoformat(),
            "promptLength": len(prompt),
            "responseLength": len(response),
            "tags": tags or [],
            **(metadata or {}),
        }
        context_file = run_dir / "context.json"
        context_file.write_text(json.dumps(context, indent=2), encoding="utf-8")
        saved_files.append(self._file_info(context_file, "context", expert_slug, run_ts))

        # Save prompts
        if prompt:
            prompt_file = run_dir / "prompt.md"
            prompt_file.write_text(prompt, encoding="utf-8")
            saved_files.append(self._file_info(prompt_file, "prompt", expert_slug, run_ts))

        if system_prompt:
            system_file = run_dir / "system.md"
            system_file.write_text(system_prompt, encoding="utf-8")
            saved_files.append(self._file_info(system_file, "system_prompt", expert_slug, run_ts))

        # Extract and save scripts into this run's folder
        script_files = self._extract_scripts(run_dir, response)
        for sf in script_files:
            saved_files.append(self._file_info(sf, "script", expert_slug, run_ts))

        logger.info(
            "Expert artifacts saved: %s/%s (%d files, %d bytes response)",
            expert_slug,
            run_ts,
            len(saved_files),
            len(response),
        )

        return {
            "runId": run_id,
            "runTs": run_ts,
            "expertSlug": expert_slug,
            "artifactDir": str(run_dir),
            "files": saved_files,
        }

    def _extract_scripts(self, run_dir: Path, response: str) -> list[Path]:
        """Extract code blocks from response and save as script files."""
        pattern = r"```(\w+)?\n(.*?)```"
        matches = re.findall(pattern, response, re.DOTALL)
        if not matches:
            return []

        scripts_dir = run_dir / "scripts"
        scripts_dir.mkdir(exist_ok=True)
        scripts: list[Path] = []

        for i, (lang, code) in enumerate(matches):
            lang = lang.lower() if lang else "txt"
            ext = _EXT_MAP.get(lang, ".txt")
            script_file = scripts_dir / f"script_{i + 1}{ext}"
            script_file.write_text(code.strip(), encoding="utf-8")

            if ext in (".sh", ".py"):
                script_file.chmod(0o755)

            scripts.append(script_file)
            logger.info("Extracted script: %s (%s, %d bytes)", script_file.name, lang, len(code))

        return scripts

    def save_artifact(self, expert_name: str, filename: str, content: str | bytes, run_ts: str | None = None) -> Path:
        """Save an arbitrary artifact file into a run folder."""
        run_ts = run_ts or datetime.now(UTC).strftime("%Y%m%d_%H%M%S")
        run_dir = self._make_run_dir(expert_name, run_ts)
        artifact_path = run_dir / filename
        artifact_path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            artifact_path.write_bytes(content)
        else:
            artifact_path.write_text(content, encoding="utf-8")
        return artifact_path

    # ── Listing ──────────────────────────────────────────────────────────────

    def list_runs(self, expert_name: str) -> list[dict[str, Any]]:
        """List all run folders for an agent, newest first."""
        expert_slug = _slugify(expert_name)
        agent_dir = OUTPUTS_ROOT / expert_slug
        if not agent_dir.exists():
            return []

        runs: list[dict[str, Any]] = []
        for run_dir in sorted(agent_dir.iterdir(), reverse=True):
            if not run_dir.is_dir() or run_dir.name.startswith(("_", ".")):
                continue
            files = self._scan_dir(run_dir, expert_slug, run_dir.name)
            total_size = sum(f["sizeBytes"] for f in files)
            runs.append(
                {
                    "runTs": run_dir.name,
                    "expertSlug": expert_slug,
                    "expertName": expert_name,
                    "artifactDir": str(run_dir),
                    "fileCount": len(files),
                    "totalSize": total_size,
                    "files": files,
                }
            )
        return runs

    def list_artifacts(self, expert_name: str | None = None, date: str | None = None) -> list[dict[str, Any]]:
        """List artifacts. Filters by expert_name and/or date prefix on run_ts."""
        artifacts: list[dict[str, Any]] = []

        if expert_name:
            slug = _slugify(expert_name)
            agent_dir = OUTPUTS_ROOT / slug
            if agent_dir.exists():
                for run_dir in sorted(agent_dir.iterdir(), reverse=True):
                    if not run_dir.is_dir() or run_dir.name.startswith(("_", ".")):
                        continue
                    if date and not run_dir.name.startswith(date.replace("-", "")):
                        continue
                    artifacts.extend(self._scan_dir(run_dir, slug, run_dir.name))
        else:
            artifacts = self.list_all_artifacts(date_filter=date)

        return artifacts

    def list_all_artifacts(self, date_filter: str | None = None) -> list[dict[str, Any]]:
        """Scan all agent directories and return a flat list of all artifacts."""
        artifacts: list[dict[str, Any]] = []

        for agent_dir in sorted(OUTPUTS_ROOT.iterdir()):
            if not agent_dir.is_dir() or agent_dir.name.startswith(("_", ".")):
                continue
            for run_dir in sorted(agent_dir.iterdir(), reverse=True):
                if not run_dir.is_dir() or run_dir.name.startswith(("_", ".")):
                    continue
                if date_filter and not run_dir.name.startswith(date_filter.replace("-", "")):
                    continue
                artifacts.extend(self._scan_dir(run_dir, agent_dir.name, run_dir.name))

        return artifacts

    def get_file_content(self, expert_name: str, run_ts: str, filename: str) -> str | None:
        """Read content of a specific file in a run folder."""
        expert_slug = _slugify(expert_name)
        file_path = OUTPUTS_ROOT / expert_slug / run_ts / filename
        if file_path.exists() and file_path.is_file():
            return file_path.read_text(encoding="utf-8")
        return None

    def cleanup(self, max_age_days: int = 30, max_total_mb: int = 500) -> dict[str, Any]:
        """Remove stale artifacts to enforce retention policy."""
        now = time.time()
        max_age_secs = max_age_days * 86_400
        max_total_bytes = max_total_mb * 1_048_576

        files_removed = 0
        bytes_freed = 0

        all_files: list[tuple[Path, os.stat_result]] = []
        for agent_dir in OUTPUTS_ROOT.iterdir():
            if not agent_dir.is_dir() or agent_dir.name.startswith(("_", ".")):
                continue
            for f in agent_dir.rglob("*"):
                if f.is_file():
                    all_files.append((f, f.stat()))

        # Phase 1: remove files older than max_age_days
        remaining: list[tuple[Path, os.stat_result]] = []
        for f, st in all_files:
            if now - st.st_mtime > max_age_secs:
                try:
                    bytes_freed += st.st_size
                    f.unlink()
                    files_removed += 1
                except OSError as exc:
                    logger.warning("Failed to remove %s: %s", f, exc)
            else:
                remaining.append((f, st))

        # Phase 2: enforce total size budget
        total_size = sum(st.st_size for _, st in remaining)
        if total_size > max_total_bytes:
            remaining.sort(key=lambda pair: pair[1].st_mtime)
            for f, st in remaining:
                if total_size <= max_total_bytes:
                    break
                try:
                    bytes_freed += st.st_size
                    f.unlink()
                    files_removed += 1
                    total_size -= st.st_size
                except OSError as exc:
                    logger.warning("Failed to remove %s: %s", f, exc)

        # Clean up empty directories
        for d in sorted(OUTPUTS_ROOT.rglob("*"), reverse=True):
            if d.is_dir() and not any(d.iterdir()):
                try:
                    d.rmdir()
                except OSError:
                    pass

        logger.info("Expert artifact cleanup: %d files removed, %d bytes freed", files_removed, bytes_freed)
        return {"files_removed": files_removed, "bytes_freed": bytes_freed}

    # ── Internal helpers ──────────────────────────────────────────────────────

    def _file_info(self, path: Path, category: str, expert_slug: str, run_ts: str) -> dict[str, Any]:
        """Build a file descriptor dict for a saved artifact."""
        stat = path.stat()
        return {
            "fileName": path.name,
            "filePath": str(path),
            "sizeBytes": stat.st_size,
            "mimeType": _detect_mime(path),
            "fileType": _detect_file_type(path.suffix),
            "category": category,
            "expertSlug": expert_slug,
            "runTs": run_ts,
            "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
        }

    def _scan_dir(self, directory: Path, expert_slug: str, run_ts: str) -> list[dict[str, Any]]:
        """Scan a directory and return file info dicts for all files."""
        artifacts: list[dict[str, Any]] = []
        for f in sorted(directory.rglob("*")):
            if f.is_file():
                try:
                    stat = f.stat()
                    rel = f.relative_to(directory)
                    artifacts.append(
                        {
                            "fileName": str(rel),
                            "filePath": str(f),
                            "sizeBytes": stat.st_size,
                            "mimeType": _detect_mime(f),
                            "fileType": _detect_file_type(f.suffix),
                            "expertSlug": expert_slug,
                            "runTs": run_ts,
                            "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
                            "modified": datetime.fromtimestamp(stat.st_mtime, tz=UTC).isoformat(),
                        }
                    )
                except OSError:
                    continue
        return artifacts


expert_artifacts = ExpertArtifacts()
