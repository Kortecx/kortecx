"""
Filesystem-based plan store for LIVE and FREEZE plan management.
Mirrors the MCP versioning pattern with configurable version limits.
"""

from __future__ import annotations

import json
import logging
import shutil
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

PLANS_ROOT = Path(__file__).resolve().parents[3] / "plans"
LIVE_DIR = PLANS_ROOT / "LIVE"
FREEZE_DIR = PLANS_ROOT / "FREEZE"

DEFAULT_MAX_VERSIONS = 3


def _ensure_dirs() -> None:
    LIVE_DIR.mkdir(parents=True, exist_ok=True)
    FREEZE_DIR.mkdir(parents=True, exist_ok=True)


def _workflow_dir(base: Path, slug: str) -> Path:
    d = base / slug
    d.mkdir(parents=True, exist_ok=True)
    return d


def _latest_version(wf_dir: Path) -> int:
    """Return the highest version number found, or 0 if none."""
    versions = sorted(wf_dir.glob("plan_v*.json"))
    if not versions:
        return 0
    try:
        return max(int(p.stem.split("_v")[1]) for p in versions)
    except (IndexError, ValueError):
        return 0


def save_live_plan(
    slug: str,
    dag: dict[str, Any],
    markdown: str | None = None,
    max_versions: int = DEFAULT_MAX_VERSIONS,
) -> dict[str, Any]:
    """Save a new plan version to LIVE, prune old versions beyond limit."""
    _ensure_dirs()
    wf_dir = _workflow_dir(LIVE_DIR, slug)

    current = _latest_version(wf_dir)
    new_version = current + 1

    json_path = wf_dir / f"plan_v{new_version}.json"
    json_path.write_text(json.dumps(dag, indent=2), encoding="utf-8")

    md_path = wf_dir / f"plan_v{new_version}.md"
    md_path.write_text(markdown or "", encoding="utf-8")

    # Prune old versions beyond limit
    _prune_versions(wf_dir, max_versions)

    logger.info("Saved LIVE plan v%d for %s (keeping %d)", new_version, slug, max_versions)
    return {"version": new_version, "path": str(json_path)}


def _prune_versions(wf_dir: Path, max_versions: int) -> None:
    """Remove oldest versions beyond the limit."""
    json_files = sorted(wf_dir.glob("plan_v*.json"), key=lambda p: p.name, reverse=True)
    for old_json in json_files[max(1, max_versions):]:
        old_json.unlink(missing_ok=True)
        old_md = old_json.with_suffix(".md")
        old_md.unlink(missing_ok=True)
        logger.info("Pruned old plan: %s", old_json.name)


def get_live_plan(slug: str) -> dict[str, Any] | None:
    """Return the latest LIVE plan for a workflow, or None."""
    _ensure_dirs()
    wf_dir = LIVE_DIR / slug
    if not wf_dir.exists():
        return None

    latest_v = _latest_version(wf_dir)
    if latest_v == 0:
        return None

    json_path = wf_dir / f"plan_v{latest_v}.json"
    md_path = wf_dir / f"plan_v{latest_v}.md"

    dag = json.loads(json_path.read_text(encoding="utf-8")) if json_path.exists() else {}
    markdown = md_path.read_text(encoding="utf-8") if md_path.exists() else ""

    return {
        "version": latest_v,
        "dag": dag,
        "markdown": markdown,
        "path": str(json_path),
    }


def list_live_versions(slug: str) -> list[dict[str, Any]]:
    """Return all LIVE versions for a workflow, newest first."""
    _ensure_dirs()
    wf_dir = LIVE_DIR / slug
    if not wf_dir.exists():
        return []

    results = []
    json_files = sorted(wf_dir.glob("plan_v*.json"), key=lambda p: p.name, reverse=True)
    for jf in json_files:
        try:
            v = int(jf.stem.split("_v")[1])
        except (IndexError, ValueError):
            continue
        results.append({
            "version": v,
            "path": str(jf),
            "modified": jf.stat().st_mtime,
        })
    return results


def get_live_plan_version(slug: str, version: int) -> dict[str, Any] | None:
    """Return a specific LIVE plan version."""
    _ensure_dirs()
    wf_dir = LIVE_DIR / slug
    json_path = wf_dir / f"plan_v{version}.json"
    md_path = wf_dir / f"plan_v{version}.md"

    if not json_path.exists():
        return None

    dag = json.loads(json_path.read_text(encoding="utf-8"))
    markdown = md_path.read_text(encoding="utf-8") if md_path.exists() else ""

    return {"version": version, "dag": dag, "markdown": markdown, "path": str(json_path)}


def freeze_plan(slug: str) -> dict[str, Any] | None:
    """Copy the latest LIVE plan to FREEZE. Returns frozen plan info or None."""
    live = get_live_plan(slug)
    if not live:
        logger.warning("No LIVE plan to freeze for %s", slug)
        return None

    _ensure_dirs()
    freeze_dir = _workflow_dir(FREEZE_DIR, slug)

    frozen_json = freeze_dir / "plan_frozen.json"
    frozen_md = freeze_dir / "plan_frozen.md"

    frozen_json.write_text(json.dumps(live["dag"], indent=2), encoding="utf-8")
    frozen_md.write_text(live.get("markdown", ""), encoding="utf-8")

    logger.info("Froze plan v%d for %s", live["version"], slug)
    return {
        "version": live["version"],
        "dag": live["dag"],
        "markdown": live.get("markdown", ""),
        "path": str(frozen_json),
    }


def refreeze_plan(slug: str, version: int | None = None) -> dict[str, Any] | None:
    """Replace FREEZE with a specific LIVE version (or latest if version is None)."""
    if version is not None:
        plan = get_live_plan_version(slug, version)
    else:
        plan = get_live_plan(slug)

    if not plan:
        logger.warning("No plan found to refreeze for %s (version=%s)", slug, version)
        return None

    _ensure_dirs()
    freeze_dir = _workflow_dir(FREEZE_DIR, slug)

    frozen_json = freeze_dir / "plan_frozen.json"
    frozen_md = freeze_dir / "plan_frozen.md"

    frozen_json.write_text(json.dumps(plan["dag"], indent=2), encoding="utf-8")
    frozen_md.write_text(plan.get("markdown", ""), encoding="utf-8")

    logger.info("Refroze plan v%d for %s", plan["version"], slug)
    return plan


def unfreeze_plan(slug: str) -> bool:
    """Remove the FREEZE snapshot for a workflow."""
    freeze_dir = FREEZE_DIR / slug
    if freeze_dir.exists():
        shutil.rmtree(freeze_dir)
        logger.info("Unfroze plan for %s", slug)
        return True
    return False


def get_frozen_plan(slug: str) -> dict[str, Any] | None:
    """Return the FREEZE plan or None."""
    frozen_json = FREEZE_DIR / slug / "plan_frozen.json"
    frozen_md = FREEZE_DIR / slug / "plan_frozen.md"

    if not frozen_json.exists():
        return None

    dag = json.loads(frozen_json.read_text(encoding="utf-8"))
    markdown = frozen_md.read_text(encoding="utf-8") if frozen_md.exists() else ""

    return {"dag": dag, "markdown": markdown, "path": str(frozen_json)}


def get_execution_plan(slug: str, is_frozen: bool) -> dict[str, Any] | None:
    """Return the FREEZE plan if frozen, otherwise the latest LIVE plan."""
    if is_frozen:
        frozen = get_frozen_plan(slug)
        if frozen:
            return frozen
        logger.warning("Frozen flag set but no FREEZE plan for %s, falling back to LIVE", slug)
    return get_live_plan(slug)
