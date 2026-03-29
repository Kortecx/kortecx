"""Expert artifacts — persists expert run outputs, scripts, and context to disk."""

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

EXPERTS_ROOT = Path(__file__).resolve().parents[3] / "agents" / "local"

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
    """Manages expert execution artifacts on disk with date-based organization."""

    def __init__(self) -> None:
        EXPERTS_ROOT.mkdir(parents=True, exist_ok=True)

    def get_artifact_dir(self, expert_name: str, date: str | None = None) -> Path:
        """Get or create the directory for an expert's artifacts on a given date."""
        date = date or datetime.now(UTC).strftime("%Y-%m-%d")
        expert_slug = _slugify(expert_name)
        artifact_dir = EXPERTS_ROOT / date / expert_slug
        artifact_dir.mkdir(parents=True, exist_ok=True)
        return artifact_dir

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
        date = datetime.now(UTC).strftime("%Y-%m-%d")
        artifact_dir = self.get_artifact_dir(expert_name, date)
        ts = datetime.now(UTC).strftime("%Y%m%d_%H%M%S")
        run_id = f"run-{ts}-{int(time.time() * 1000) % 100000}"

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
        response_file = artifact_dir / f"response_{ts}.md"
        response_file.write_text(frontmatter + response, encoding="utf-8")
        saved_files.append(self._file_info(response_file, "response", expert_name, date))

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
        context_file = artifact_dir / f"context_{ts}.json"
        context_file.write_text(json.dumps(context, indent=2), encoding="utf-8")
        saved_files.append(self._file_info(context_file, "context", expert_name, date))

        # Save prompts
        if prompt:
            prompt_file = artifact_dir / f"prompt_{ts}.md"
            prompt_file.write_text(prompt, encoding="utf-8")
            saved_files.append(self._file_info(prompt_file, "prompt", expert_name, date))

        if system_prompt:
            system_file = artifact_dir / f"system_{ts}.md"
            system_file.write_text(system_prompt, encoding="utf-8")
            saved_files.append(self._file_info(system_file, "system_prompt", expert_name, date))

        # Extract and save scripts
        script_files = self.extract_and_save_scripts(expert_name, response, date)
        for sf in script_files:
            saved_files.append(self._file_info(sf, "script", expert_name, date))

        logger.info(
            "Expert artifacts saved: %s/%s (%d files, %d bytes response)",
            date,
            _slugify(expert_name),
            len(saved_files),
            len(response),
        )

        return {
            "runId": run_id,
            "date": date,
            "expertSlug": _slugify(expert_name),
            "artifactDir": str(artifact_dir),
            "files": saved_files,
        }

    def extract_and_save_scripts(self, expert_name: str, response: str, date: str | None = None) -> list[Path]:
        """Extract code blocks from response and save as script files."""
        artifact_dir = self.get_artifact_dir(expert_name, date)
        scripts_dir = artifact_dir / "scripts"

        pattern = r"```(\w+)?\n(.*?)```"
        matches = re.findall(pattern, response, re.DOTALL)
        if not matches:
            return []

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

    def save_artifact(self, expert_name: str, filename: str, content: str | bytes, date: str | None = None) -> Path:
        """Save an arbitrary artifact file."""
        artifact_dir = self.get_artifact_dir(expert_name, date)
        artifact_path = artifact_dir / filename
        artifact_path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            artifact_path.write_bytes(content)
        else:
            artifact_path.write_text(content, encoding="utf-8")
        return artifact_path

    def list_artifacts(self, expert_name: str | None = None, date: str | None = None) -> list[dict[str, Any]]:
        """List artifacts for a specific expert and/or date."""
        artifacts: list[dict[str, Any]] = []

        if date and expert_name:
            # Specific expert on specific date
            artifact_dir = EXPERTS_ROOT / date / _slugify(expert_name)
            if artifact_dir.exists():
                artifacts.extend(self._scan_dir(artifact_dir, expert_name, date))
        elif date:
            # All experts on a specific date
            date_dir = EXPERTS_ROOT / date
            if date_dir.exists():
                for expert_dir in sorted(date_dir.iterdir()):
                    if expert_dir.is_dir():
                        artifacts.extend(self._scan_dir(expert_dir, expert_dir.name, date))
        elif expert_name:
            # Specific expert across all dates
            slug = _slugify(expert_name)
            for date_dir in sorted(EXPERTS_ROOT.iterdir()):
                if date_dir.is_dir() and re.match(r"\d{4}-\d{2}-\d{2}", date_dir.name):
                    expert_dir = date_dir / slug
                    if expert_dir.exists():
                        artifacts.extend(self._scan_dir(expert_dir, expert_name, date_dir.name))
        else:
            # Everything
            artifacts = self.list_all_artifacts()

        return artifacts

    def list_all_artifacts(self) -> list[dict[str, Any]]:
        """Scan all date directories and return a flat list of all expert artifacts."""
        artifacts: list[dict[str, Any]] = []

        for date_dir in sorted(EXPERTS_ROOT.iterdir()):
            if not date_dir.is_dir() or not re.match(r"\d{4}-\d{2}-\d{2}", date_dir.name):
                continue
            date = date_dir.name
            for expert_dir in sorted(date_dir.iterdir()):
                if expert_dir.is_dir():
                    artifacts.extend(self._scan_dir(expert_dir, expert_dir.name, date))

        return artifacts

    def cleanup(self, max_age_days: int = 30, max_total_mb: int = 500) -> dict[str, Any]:
        """Remove stale artifacts to enforce retention policy."""
        now = time.time()
        max_age_secs = max_age_days * 86_400
        max_total_bytes = max_total_mb * 1_048_576

        files_removed = 0
        bytes_freed = 0

        all_files: list[tuple[Path, os.stat_result]] = []
        for date_dir in EXPERTS_ROOT.iterdir():
            if not date_dir.is_dir() or not re.match(r"\d{4}-\d{2}-\d{2}", date_dir.name):
                continue
            for f in date_dir.rglob("*"):
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
        for d in sorted(EXPERTS_ROOT.rglob("*"), reverse=True):
            if d.is_dir() and not any(d.iterdir()):
                try:
                    d.rmdir()
                except OSError:
                    pass

        logger.info("Expert artifact cleanup: %d files removed, %d bytes freed", files_removed, bytes_freed)
        return {"files_removed": files_removed, "bytes_freed": bytes_freed}

    # ── Internal helpers ──────────────────────────────────────────────────────

    def _file_info(self, path: Path, category: str, expert_name: str, date: str) -> dict[str, Any]:
        """Build a file descriptor dict for a saved artifact."""
        stat = path.stat()
        return {
            "fileName": path.name,
            "filePath": str(path),
            "sizeBytes": stat.st_size,
            "mimeType": _detect_mime(path),
            "fileType": _detect_file_type(path.suffix),
            "category": category,
            "expertName": expert_name,
            "expertSlug": _slugify(expert_name),
            "date": date,
            "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
        }

    def _scan_dir(self, directory: Path, expert_name: str, date: str) -> list[dict[str, Any]]:
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
                            "expertName": expert_name,
                            "date": date,
                            "createdAt": datetime.fromtimestamp(stat.st_ctime, tz=UTC).isoformat(),
                            "modified": datetime.fromtimestamp(stat.st_mtime, tz=UTC).isoformat(),
                        }
                    )
                except OSError:
                    continue
        return artifacts


expert_artifacts = ExpertArtifacts()
