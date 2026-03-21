"""Expert manager — loads, versions, and manages expert definitions on disk."""

from __future__ import annotations

import json
import logging
import shutil
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

logger = logging.getLogger("engine.expert_manager")

EXPERTS_ROOT = Path(__file__).resolve().parents[3] / "experts"
MARKETPLACE_DIR = EXPERTS_ROOT / "marketplace"
LOCAL_DIR = EXPERTS_ROOT / "local"


class ExpertManager:
    """Manages expert definitions stored on disk with per-file versioning."""

    def __init__(self) -> None:
        self._cache: dict[str, dict[str, Any]] = {}
        MARKETPLACE_DIR.mkdir(parents=True, exist_ok=True)
        LOCAL_DIR.mkdir(parents=True, exist_ok=True)

    # ── Loading ──────────────────────────────────────────────────────────────

    def load_all(self) -> list[dict[str, Any]]:
        """Load all experts from marketplace and local directories."""
        experts: list[dict[str, Any]] = []
        for source, base_dir in [("marketplace", MARKETPLACE_DIR), ("local", LOCAL_DIR)]:
            for expert_dir in sorted(base_dir.iterdir()):
                if not expert_dir.is_dir() or expert_dir.name.startswith(("_", ".")):
                    continue
                expert = self._load_expert(expert_dir, source)
                if expert:
                    experts.append(expert)
                    self._cache[expert["id"]] = expert
        return experts

    def get(self, expert_id: str) -> dict[str, Any] | None:
        """Get a single expert by ID."""
        if expert_id in self._cache:
            return self._cache[expert_id]
        self.load_all()
        return self._cache.get(expert_id)

    def get_prompt(self, expert_id: str, prompt_type: str = "system") -> str:
        """Get system.md or user.md content for an expert."""
        expert = self.get(expert_id)
        if not expert:
            return ""
        expert_dir = Path(expert["_dir"])
        prompt_file = expert_dir / f"{prompt_type}.md"
        if prompt_file.exists():
            return prompt_file.read_text(encoding="utf-8")
        return ""

    # ── CRUD ─────────────────────────────────────────────────────────────────

    def create_local(self, name: str, role: str, config: dict[str, Any]) -> dict[str, Any]:
        """Create a new local expert with initial files."""
        slug = name.lower().replace(" ", "-").replace("_", "-")
        slug = "".join(c for c in slug if c.isalnum() or c == "-")
        expert_id = f"local-{slug}"
        expert_dir = LOCAL_DIR / slug
        expert_dir.mkdir(parents=True, exist_ok=True)

        # Create expert.json
        expert_data: dict[str, Any] = {
            "id": expert_id,
            "name": name,
            "description": config.get("description", ""),
            "role": role,
            "version": "1.0.0",
            "modelSource": config.get("modelSource", "local"),
            "localModelConfig": config.get(
                "localModelConfig",
                {"engine": "ollama", "modelName": "llama3.2:3b"},
            ),
            "temperature": config.get("temperature", 0.7),
            "maxTokens": config.get("maxTokens", 4096),
            "tags": config.get("tags", []),
            "capabilities": config.get("capabilities", []),
            "isPublic": config.get("isPublic", False),
            "category": config.get("category", "custom"),
            "createdAt": datetime.now(UTC).isoformat(),
            "updatedAt": datetime.now(UTC).isoformat(),
        }
        (expert_dir / "expert.json").write_text(
            json.dumps(expert_data, indent=2),
            encoding="utf-8",
        )

        # Create prompt files
        system_prompt = config.get(
            "systemPrompt",
            f"You are {name}, a specialized AI expert with role: {role}.",
        )
        (expert_dir / "system.md").write_text(system_prompt, encoding="utf-8")

        user_prompt = config.get(
            "userPrompt",
            "## Task\n{{task}}\n\n## Context\n{{context}}\n\n## Constraints\n{{constraints}}",
        )
        (expert_dir / "user.md").write_text(user_prompt, encoding="utf-8")

        # Create README
        (expert_dir / "README.md").write_text(
            f"# {name}\n\n{expert_data['description']}\n\n**Role:** {role}\n**Category:** {expert_data['category']}\n",
            encoding="utf-8",
        )

        # Create versions directory
        (expert_dir / ".versions").mkdir(exist_ok=True)

        # Update local registry
        self._update_registry(LOCAL_DIR)

        # Cache
        expert_data["_dir"] = str(expert_dir)
        expert_data["_source"] = "local"
        self._cache[expert_id] = expert_data

        return expert_data

    def update_file(self, expert_id: str, filename: str, content: str) -> dict[str, Any]:
        """Update a single file in an expert, creating a version of ONLY that file."""
        expert = self.get(expert_id)
        if not expert:
            msg = f"Expert {expert_id} not found"
            raise ValueError(msg)

        expert_dir = Path(expert["_dir"])
        file_path = expert_dir / filename
        file_path.parent.mkdir(parents=True, exist_ok=True)
        versions_dir = expert_dir / ".versions"
        versions_dir.mkdir(exist_ok=True)

        # Version the old file if it exists and content changed
        if file_path.exists():
            old_content = file_path.read_text(encoding="utf-8")
            if old_content != content:
                ts = int(time.time() * 1000)
                version_name = f"{filename}.v{ts}"
                (versions_dir / version_name).write_text(old_content, encoding="utf-8")
                logger.info("Versioned %s → %s", filename, version_name)

        # Write new content
        file_path.write_text(content, encoding="utf-8")

        # If expert.json changed, bump patch version
        if filename == "expert.json":
            data = json.loads(content)
            v = data.get("version", "1.0.0").split(".")
            v[-1] = str(int(v[-1]) + 1)
            data["version"] = ".".join(v)
            data["updatedAt"] = datetime.now(UTC).isoformat()
            file_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
            self._cache[expert_id] = {**expert, **data}
        else:
            # Touch updatedAt in expert.json
            ej = expert_dir / "expert.json"
            if ej.exists():
                edata = json.loads(ej.read_text(encoding="utf-8"))
                edata["updatedAt"] = datetime.now(UTC).isoformat()
                ej.write_text(json.dumps(edata, indent=2), encoding="utf-8")

        return {"file": filename, "versioned": True, "expert_id": expert_id}

    def delete_expert(self, expert_id: str) -> bool:
        """Delete a local expert. Marketplace experts cannot be deleted."""
        expert = self.get(expert_id)
        if not expert:
            return False
        if expert.get("_source") == "marketplace":
            msg = "Cannot delete marketplace experts"
            raise ValueError(msg)

        expert_dir = Path(expert["_dir"])
        if expert_dir.exists():
            shutil.rmtree(expert_dir)
        self._cache.pop(expert_id, None)
        self._update_registry(LOCAL_DIR)
        return True

    # ── Versioning ───────────────────────────────────────────────────────────

    def get_versions(self, expert_id: str, filename: str) -> list[dict[str, Any]]:
        """List all versions of a specific file."""
        expert = self.get(expert_id)
        if not expert:
            return []

        versions_dir = Path(expert["_dir"]) / ".versions"
        if not versions_dir.exists():
            return []

        prefix = f"{filename}.v"
        versions: list[dict[str, Any]] = []
        for vf in sorted(versions_dir.iterdir(), reverse=True):
            if vf.name.startswith(prefix):
                ts = int(vf.name.split(".v")[-1])
                versions.append(
                    {
                        "filename": vf.name,
                        "timestamp": ts,
                        "date": datetime.fromtimestamp(ts / 1000, tz=UTC).isoformat(),
                        "size": vf.stat().st_size,
                    }
                )
        return versions

    def restore_version(self, expert_id: str, version_filename: str) -> dict[str, Any]:
        """Restore a file from a specific version."""
        expert = self.get(expert_id)
        if not expert:
            msg = f"Expert {expert_id} not found"
            raise ValueError(msg)

        versions_dir = Path(expert["_dir"]) / ".versions"
        version_file = versions_dir / version_filename

        if not version_file.exists():
            msg = f"Version {version_filename} not found"
            raise ValueError(msg)

        # Extract original filename
        original_name = version_filename.rsplit(".v", 1)[0]
        content = version_file.read_text(encoding="utf-8")

        # This will create a new version of the current file before overwriting
        return self.update_file(expert_id, original_name, content)

    # ── File listing ─────────────────────────────────────────────────────────

    def list_files(self, expert_id: str) -> list[dict[str, Any]]:
        """List all files in an expert directory."""
        expert = self.get(expert_id)
        if not expert:
            return []

        expert_dir = Path(expert["_dir"])
        files: list[dict[str, Any]] = []
        for f in sorted(expert_dir.iterdir()):
            if f.name.startswith(".") or f.is_dir():
                continue
            files.append(
                {
                    "name": f.name,
                    "size": f.stat().st_size,
                    "modified": datetime.fromtimestamp(f.stat().st_mtime, tz=UTC).isoformat(),
                }
            )
        return files

    # ── Internal helpers ─────────────────────────────────────────────────────

    def _load_expert(self, expert_dir: Path, source: str) -> dict[str, Any] | None:
        """Load a single expert from its directory."""
        ej = expert_dir / "expert.json"
        if not ej.exists():
            return None
        try:
            data: dict[str, Any] = json.loads(ej.read_text(encoding="utf-8"))
            data["_dir"] = str(expert_dir)
            data["_source"] = source
            data["hasSystemPrompt"] = (expert_dir / "system.md").exists()
            data["hasUserPrompt"] = (expert_dir / "user.md").exists()
            return data
        except Exception:
            logger.exception("Failed to load expert from %s", expert_dir)
            return None

    def _update_registry(self, base_dir: Path) -> None:
        """Rebuild the _registry.json for a directory."""
        registry: dict[str, Any] = {"version": "1.0.0", "experts": []}
        for expert_dir in sorted(base_dir.iterdir()):
            if not expert_dir.is_dir() or expert_dir.name.startswith(("_", ".")):
                continue
            ej = expert_dir / "expert.json"
            if ej.exists():
                try:
                    data = json.loads(ej.read_text(encoding="utf-8"))
                    registry["experts"].append(
                        {
                            "id": data.get("id"),
                            "dir": expert_dir.name,
                            "name": data.get("name"),
                            "role": data.get("role"),
                            "category": data.get("category", "custom"),
                            "version": data.get("version", "1.0.0"),
                        }
                    )
                except Exception:
                    pass
        (base_dir / "_registry.json").write_text(
            json.dumps(registry, indent=2),
            encoding="utf-8",
        )


# Singleton instance
expert_manager = ExpertManager()
