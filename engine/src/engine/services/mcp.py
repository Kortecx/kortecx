from __future__ import annotations

import logging
import os
import json
import subprocess
import asyncio
import uuid
from pathlib import Path
from dataclasses import dataclass, field, asdict
from typing import Any

logger = logging.getLogger("engine.mcp")

# Directories
MCP_PREBUILT_DIR = Path(__file__).resolve().parents[3] / "mcp"
MCP_SCRIPTS_DIR = Path(__file__).resolve().parents[3] / "mcp_scripts"
MCP_VERSIONS_DIR = Path(__file__).resolve().parents[3] / "mcp_scripts" / ".versions"
MCP_PROMPTS_DIR = Path(__file__).resolve().parents[3] / "mcp" / "prompts"

DEFAULT_MAX_VERSIONS = 3


@dataclass
class McpServer:
    id: str
    name: str
    description: str
    language: str  # python | typescript | javascript
    filename: str
    source: str  # prebuilt | generated | persisted
    code: str
    status: str = "idle"  # idle | running | error | tested
    test_output: str = ""
    created_at: str = ""
    prompt: str = ""  # original generation prompt
    is_public: bool = False
    generation_time_ms: int = 0
    cpu_percent: float = 0.0

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class McpService:
    """Manages MCP server scripts — discovery, caching, testing, persistence."""

    def __init__(self) -> None:
        self._cache: dict[str, McpServer] = {}
        self.max_versions: int = DEFAULT_MAX_VERSIONS
        MCP_PREBUILT_DIR.mkdir(parents=True, exist_ok=True)
        MCP_SCRIPTS_DIR.mkdir(parents=True, exist_ok=True)
        MCP_VERSIONS_DIR.mkdir(parents=True, exist_ok=True)
        MCP_PROMPTS_DIR.mkdir(parents=True, exist_ok=True)

    def list_prebuilt(self) -> list[dict[str, Any]]:
        """Discover prebuilt MCP scripts in engine/mcp/."""
        servers: list[dict[str, Any]] = []
        for f in sorted(MCP_PREBUILT_DIR.iterdir()):
            if f.suffix in (".py", ".ts", ".js", ".mjs"):
                lang = "python" if f.suffix == ".py" else "typescript" if f.suffix == ".ts" else "javascript"
                desc = ""
                code = f.read_text(encoding="utf-8")
                # Extract description from first docstring or comment
                for line in code.splitlines()[:10]:
                    stripped = line.strip()
                    if stripped.startswith("#") or stripped.startswith("//"):
                        desc = stripped.lstrip("#/ ").strip()
                        break
                    if stripped.startswith('"""') or stripped.startswith("'''"):
                        desc = stripped.strip("\"' ").strip()
                        break
                servers.append(McpServer(
                    id=f"prebuilt-{f.stem}",
                    name=f.stem.replace("_", " ").replace("-", " ").title(),
                    description=desc or f"Prebuilt MCP server: {f.name}",
                    language=lang,
                    filename=f.name,
                    source="prebuilt",
                    code=code,
                    status="idle",
                ).to_dict())
        return servers

    def list_persisted(self) -> list[dict[str, Any]]:
        """List persisted user MCP scripts from engine/mcp_scripts/."""
        servers: list[dict[str, Any]] = []
        for f in sorted(MCP_SCRIPTS_DIR.iterdir()):
            if f.suffix in (".py", ".ts", ".js", ".mjs"):
                lang = "python" if f.suffix == ".py" else "typescript" if f.suffix == ".ts" else "javascript"
                code = f.read_text(encoding="utf-8")
                # Try to read metadata sidecar
                meta_path = f.with_suffix(f.suffix + ".meta.json")
                meta: dict[str, str] = {}
                if meta_path.exists():
                    try:
                        meta = json.loads(meta_path.read_text(encoding="utf-8"))
                    except Exception:
                        pass
                servers.append(McpServer(
                    id=f"persisted-{f.stem}",
                    name=meta.get("name", f.stem.replace("_", " ").replace("-", " ").title()),
                    description=meta.get("description", f"User MCP server: {f.name}"),
                    language=lang,
                    filename=f.name,
                    source="persisted",
                    code=code,
                    status="idle",
                    prompt=meta.get("prompt", ""),
                    is_public=meta.get("is_public", False),
                    generation_time_ms=meta.get("generation_time_ms", 0),
                    cpu_percent=meta.get("cpu_percent", 0.0),
                ).to_dict())
        return servers

    def list_cached(self) -> list[dict[str, Any]]:
        """List session-cached (temporary) MCP scripts."""
        return [s.to_dict() for s in self._cache.values()]

    def cache_script(
        self,
        name: str,
        description: str,
        language: str,
        code: str,
        filename: str | None = None,
        prompt: str = "",
        generation_time_ms: int = 0,
        cpu_percent: float = 0.0,
        prompt_type: str = "mcp",
    ) -> dict[str, Any]:
        """Cache an MCP script in session memory (not persisted yet)."""
        sid = f"cached-{uuid.uuid4().hex[:12]}"
        ext = ".py" if language == "python" else ".ts" if language == "typescript" else ".js"
        fname = filename or f"{name.lower().replace(' ', '_')}{ext}"
        server = McpServer(
            id=sid,
            name=name,
            description=description,
            language=language,
            filename=fname,
            source="generated",
            code=code,
            status="idle",
            created_at=__import__("datetime").datetime.now(__import__("datetime").timezone.utc).isoformat(),
            prompt=prompt,
            generation_time_ms=generation_time_ms,
            cpu_percent=cpu_percent,
        )
        self._cache[sid] = server

        # Save prompt to cache/prompts/{type}/
        if prompt:
            self._save_prompt(sid, fname, prompt, prompt_type)

        logger.info("Cached MCP script: %s (%s)", name, sid)
        return server.to_dict()

    def _save_prompt(self, script_id: str, filename: str, prompt: str, prompt_type: str = "mcp") -> None:
        """Save the generation prompt to cache/prompts/{type}/."""
        cache_dir = Path(__file__).resolve().parents[3] / "cache" / "prompts" / prompt_type
        cache_dir.mkdir(parents=True, exist_ok=True)
        stem = Path(filename).stem
        prompt_file = cache_dir / f"{stem}.prompt.md"
        prompt_file.write_text(
            f"# Generation Prompt ({prompt_type})\n\n"
            f"**Script:** {filename}\n"
            f"**ID:** {script_id}\n"
            f"**Type:** {prompt_type}\n"
            f"**Generated:** {__import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat()}\n\n"
            f"## Prompt\n\n{prompt}\n",
            encoding="utf-8",
        )
        # Also keep a copy in the legacy mcp/prompts/ dir for MCP type
        if prompt_type == "mcp":
            legacy = MCP_PROMPTS_DIR / f"{stem}.prompt.md"
            legacy.write_text(prompt_file.read_text(encoding="utf-8"), encoding="utf-8")

    def get_cached(self, script_id: str) -> dict[str, Any] | None:
        """Get a cached script by ID."""
        s = self._cache.get(script_id)
        return s.to_dict() if s else None

    def update_cached(
        self,
        script_id: str,
        code: str | None = None,
        description: str | None = None,
        is_public: bool | None = None,
    ) -> dict[str, Any] | None:
        """Update fields of a cached script."""
        s = self._cache.get(script_id)
        if not s:
            return None
        if code is not None:
            s.code = code
            s.status = "idle"
        if description is not None:
            s.description = description
        if is_public is not None:
            s.is_public = is_public
        return s.to_dict()

    def delete_cached(self, script_id: str) -> bool:
        """Remove a cached script from session."""
        return self._cache.pop(script_id, None) is not None

    async def test_script(self, script_id: str) -> dict[str, Any]:
        """Execute a cached MCP script in a subprocess to test it."""
        s = self._cache.get(script_id)
        if not s:
            return {"success": False, "error": "Script not found in cache"}

        s.status = "running"
        ext = ".py" if s.language == "python" else ".ts" if s.language == "typescript" else ".js"
        tmp_path = MCP_SCRIPTS_DIR / f".tmp_test_{script_id}{ext}"

        try:
            tmp_path.write_text(s.code, encoding="utf-8")

            if s.language == "python":
                cmd = ["python", str(tmp_path)]
            elif s.language == "typescript":
                cmd = ["npx", "tsx", str(tmp_path)]
            else:
                cmd = ["node", str(tmp_path)]

            proc = await asyncio.create_subprocess_exec(
                *cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
                cwd=str(MCP_SCRIPTS_DIR),
            )
            try:
                stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=30)
            except asyncio.TimeoutError:
                proc.kill()
                await proc.communicate()
                s.status = "error"
                s.test_output = "Timeout: script took longer than 30 seconds"
                return {"success": False, "error": s.test_output, "server": s.to_dict()}

            output = stdout.decode("utf-8", errors="replace")
            err_output = stderr.decode("utf-8", errors="replace")

            if proc.returncode == 0:
                s.status = "tested"
                s.test_output = output or "(no output)"
                return {"success": True, "output": s.test_output, "server": s.to_dict()}
            else:
                s.status = "error"
                s.test_output = err_output or output or f"Exit code {proc.returncode}"
                return {"success": False, "error": s.test_output, "server": s.to_dict()}

        except Exception as exc:
            s.status = "error"
            s.test_output = str(exc)
            return {"success": False, "error": str(exc), "server": s.to_dict()}
        finally:
            tmp_path.unlink(missing_ok=True)

    def persist_script(self, script_id: str) -> dict[str, Any] | None:
        """Persist a cached script to engine/mcp_scripts/ with versioning. Keeps cache copy."""
        s = self._cache.get(script_id)
        if not s:
            return None

        dest = MCP_SCRIPTS_DIR / s.filename

        # Save a version of the existing file before overwriting
        if dest.exists():
            self._save_version(dest)

        dest.write_text(s.code, encoding="utf-8")

        # Write metadata sidecar
        meta_path = dest.with_suffix(dest.suffix + ".meta.json")
        meta_path.write_text(json.dumps({
            "name": s.name,
            "description": s.description,
            "language": s.language,
            "created_at": s.created_at,
            "prompt": s.prompt,
            "is_public": s.is_public,
            "generation_time_ms": s.generation_time_ms,
            "cpu_percent": s.cpu_percent,
        }, indent=2), encoding="utf-8")

        s.source = "persisted"
        s.id = f"persisted-{dest.stem}"
        logger.info("Persisted MCP script: %s → %s", s.name, dest)

        # Keep in cache — user must explicitly delete cached copy
        return s.to_dict()

    def _save_version(self, file_path: Path) -> None:
        """Save a timestamped version of a file before overwriting. Prune to max_versions."""
        import datetime as _dt

        stem = file_path.stem
        suffix = file_path.suffix
        version_dir = MCP_VERSIONS_DIR / stem
        version_dir.mkdir(parents=True, exist_ok=True)

        ts = _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%d_%H%M%S")
        version_file = version_dir / f"{stem}_{ts}{suffix}"
        version_file.write_text(file_path.read_text(encoding="utf-8"), encoding="utf-8")

        # Also copy the meta sidecar if present
        meta_src = file_path.with_suffix(suffix + ".meta.json")
        if meta_src.exists():
            meta_dest = version_dir / f"{stem}_{ts}{suffix}.meta.json"
            meta_dest.write_text(meta_src.read_text(encoding="utf-8"), encoding="utf-8")

        # Prune old versions — keep only max_versions most recent
        versions = sorted(version_dir.glob(f"{stem}_*{suffix}"), key=lambda p: p.name, reverse=True)
        for old in versions[self.max_versions:]:
            old.unlink(missing_ok=True)
            old_meta = old.with_suffix(old.suffix + ".meta.json")
            old_meta.unlink(missing_ok=True)
        logger.info("Saved version: %s (keeping %d)", version_file.name, self.max_versions)

    def set_max_versions(self, n: int) -> int:
        """Set the maximum number of versions to keep. Returns the new value."""
        self.max_versions = max(1, n)
        return self.max_versions

    def list_versions(self, script_id: str) -> list[dict[str, str]]:
        """List available versions for a persisted script."""
        stem = script_id.replace("persisted-", "", 1)
        version_dir = MCP_VERSIONS_DIR / stem
        if not version_dir.exists():
            return []
        versions = []
        for f in sorted(version_dir.iterdir(), key=lambda p: p.name, reverse=True):
            if f.suffix in (".py", ".ts", ".js", ".mjs") and not f.name.endswith(".meta.json"):
                versions.append({
                    "filename": f.name,
                    "timestamp": f.name.rsplit("_", 2)[-1].split(".")[0] if "_" in f.name else "",
                    "size_bytes": f.stat().st_size,
                })
        return versions

    def delete_persisted(self, script_id: str) -> bool:
        """Delete a persisted script file."""
        stem = script_id.replace("persisted-", "", 1)
        for f in MCP_SCRIPTS_DIR.iterdir():
            if f.stem == stem and f.suffix in (".py", ".ts", ".js", ".mjs"):
                f.unlink()
                meta = f.with_suffix(f.suffix + ".meta.json")
                meta.unlink(missing_ok=True)
                return True
        return False


mcp_service = McpService()
