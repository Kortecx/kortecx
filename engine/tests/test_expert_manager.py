"""Tests for expert manager — loading, CRUD, versioning."""

from __future__ import annotations

import json

import pytest

import engine.services.expert_manager as em


@pytest.fixture
def temp_experts(tmp_path, monkeypatch):
    """Create temp expert dirs and patch module paths."""
    mp = tmp_path / "marketplace"
    lp = tmp_path / "local"
    mp.mkdir()
    lp.mkdir()
    (mp / "_registry.json").write_text('{"version":"1.0.0","experts":[]}')
    (lp / "_registry.json").write_text('{"version":"1.0.0","experts":[]}')

    monkeypatch.setattr(em, "MARKETPLACE_DIR", mp)
    monkeypatch.setattr(em, "LOCAL_DIR", lp)

    # Create a test marketplace expert
    expert_dir = mp / "test-expert"
    expert_dir.mkdir()
    (expert_dir / "expert.json").write_text(
        json.dumps(
            {
                "id": "marketplace-test-expert",
                "name": "Test Expert",
                "role": "coder",
                "version": "1.0.0",
                "temperature": 0.5,
                "maxTokens": 4096,
                "tags": ["test"],
                "category": "engineering",
            }
        )
    )
    (expert_dir / "system.md").write_text("You are a test expert.")
    (expert_dir / "user.md").write_text("## Task\n{{task}}")

    mgr = em.ExpertManager()
    return mgr, mp, lp


class TestExpertManagerLoad:
    def test_loads_marketplace_experts(self, temp_experts):
        mgr, _, _ = temp_experts
        experts = mgr.load_all()
        assert len(experts) == 1
        assert experts[0]["name"] == "Test Expert"
        assert experts[0]["_source"] == "marketplace"

    def test_get_by_id(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        expert = mgr.get("marketplace-test-expert")
        assert expert is not None
        assert expert["role"] == "coder"

    def test_get_nonexistent(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        assert mgr.get("nonexistent") is None

    def test_get_prompt(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        system = mgr.get_prompt("marketplace-test-expert", "system")
        assert "test expert" in system.lower()

    def test_get_user_prompt(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        user = mgr.get_prompt("marketplace-test-expert", "user")
        assert "{{task}}" in user

    def test_get_prompt_nonexistent_expert(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        assert mgr.get_prompt("nonexistent", "system") == ""

    def test_get_prompt_missing_file(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        assert mgr.get_prompt("marketplace-test-expert", "nonexistent") == ""

    def test_list_files(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        files = mgr.list_files("marketplace-test-expert")
        names = [f["name"] for f in files]
        assert "expert.json" in names
        assert "system.md" in names

    def test_list_files_nonexistent(self, temp_experts):
        mgr, _, _ = temp_experts
        assert mgr.list_files("nonexistent") == []

    def test_has_prompt_flags(self, temp_experts):
        mgr, _, _ = temp_experts
        experts = mgr.load_all()
        assert experts[0]["hasSystemPrompt"] is True
        assert experts[0]["hasUserPrompt"] is True

    def test_skips_hidden_and_underscore_dirs(self, temp_experts):
        mgr, mp, _ = temp_experts
        (mp / ".hidden").mkdir()
        (mp / "_internal").mkdir()
        experts = mgr.load_all()
        assert len(experts) == 1

    def test_skips_dirs_without_expert_json(self, temp_experts):
        mgr, mp, _ = temp_experts
        (mp / "no-json").mkdir()
        experts = mgr.load_all()
        assert len(experts) == 1


class TestExpertManagerCreate:
    def test_create_local_expert(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local(
            "My Expert",
            "researcher",
            {
                "description": "A test researcher",
                "systemPrompt": "Research stuff.",
                "temperature": 0.6,
            },
        )
        assert expert["id"] == "local-my-expert"
        assert expert["role"] == "researcher"
        assert (lp / "my-expert" / "expert.json").exists()
        assert (lp / "my-expert" / "system.md").exists()

    def test_create_updates_registry(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("Test Create", "writer", {})
        registry = json.loads((lp / "_registry.json").read_text())
        assert len(registry["experts"]) == 1

    def test_create_sets_defaults(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Defaults Test", "coder", {})
        assert expert["temperature"] == 0.7
        assert expert["maxTokens"] == 4096

    def test_create_writes_system_prompt(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("Prompt Test", "coder", {"systemPrompt": "Custom system."})
        content = (lp / "prompt-test" / "system.md").read_text()
        assert content == "Custom system."

    def test_create_writes_user_prompt(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("User Prompt", "coder", {"userPrompt": "Custom user."})
        content = (lp / "user-prompt" / "user.md").read_text()
        assert content == "Custom user."

    def test_create_writes_readme(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("Readme Test", "coder", {"description": "A description."})
        assert (lp / "readme-test" / "README.md").exists()

    def test_create_creates_versions_dir(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("Versions Dir", "coder", {})
        assert (lp / "versions-dir" / ".versions").is_dir()

    def test_create_caches_expert(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Cached", "coder", {})
        assert mgr.get(expert["id"]) is not None

    def test_create_default_system_prompt_includes_name(self, temp_experts):
        mgr, _, lp = temp_experts
        mgr.create_local("Named Expert", "analyst", {})
        content = (lp / "named-expert" / "system.md").read_text()
        assert "Named Expert" in content
        assert "analyst" in content


class TestExpertManagerVersioning:
    def test_update_file_creates_version(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Version Test", "analyst", {"systemPrompt": "Original."})
        mgr.update_file(expert["id"], "system.md", "Updated prompt.")

        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) >= 1

    def test_update_preserves_old_content(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Preserve Test", "writer", {"systemPrompt": "First version."})
        mgr.update_file(expert["id"], "system.md", "Second version.")

        versions_dir = lp / "preserve-test" / ".versions"
        version_files = list(versions_dir.glob("system.md.v*"))
        assert len(version_files) == 1
        assert "First version." in version_files[0].read_text()

    def test_restore_version(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Restore Test", "coder", {"systemPrompt": "Original."})
        mgr.update_file(expert["id"], "system.md", "Changed.")

        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) >= 1

        mgr.restore_version(expert["id"], versions[0]["filename"])
        content = (lp / "restore-test" / "system.md").read_text()
        assert content == "Original."

    def test_no_version_if_content_unchanged(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Same Content", "coder", {"systemPrompt": "Same."})
        mgr.update_file(expert["id"], "system.md", "Same.")
        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) == 0

    def test_update_nonexistent_expert_raises(self, temp_experts):
        mgr, _, _ = temp_experts
        with pytest.raises(ValueError, match="not found"):
            mgr.update_file("nonexistent", "system.md", "content")

    def test_restore_nonexistent_expert_raises(self, temp_experts):
        mgr, _, _ = temp_experts
        with pytest.raises(ValueError, match="not found"):
            mgr.restore_version("nonexistent", "system.md.v123")

    def test_restore_nonexistent_version_raises(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Restore Fail", "coder", {})
        with pytest.raises(ValueError, match="not found"):
            mgr.restore_version(expert["id"], "system.md.v999999")

    def test_get_versions_nonexistent_expert(self, temp_experts):
        mgr, _, _ = temp_experts
        assert mgr.get_versions("nonexistent", "system.md") == []

    def test_update_expert_json_bumps_version(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Bump Test", "coder", {})
        expert_json = json.loads((lp / "bump-test" / "expert.json").read_text())
        assert expert_json["version"] == "1.0.0"
        mgr.update_file(expert["id"], "expert.json", json.dumps({**expert_json, "description": "Updated"}))
        updated = json.loads((lp / "bump-test" / "expert.json").read_text())
        assert updated["version"] == "1.0.1"

    def test_multiple_updates_create_multiple_versions(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Multi Version", "coder", {"systemPrompt": "v1"})
        import time

        mgr.update_file(expert["id"], "system.md", "v2")
        time.sleep(0.01)  # Ensure different timestamps
        mgr.update_file(expert["id"], "system.md", "v3")
        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) == 2


class TestExpertManagerVersionPruning:
    def test_prune_versions_removes_oldest(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Prune Test", "coder", {"systemPrompt": "v1"})
        import time

        # Create several versions
        for i in range(2, 7):
            mgr.update_file(expert["id"], "system.md", f"v{i}")
            time.sleep(0.01)

        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) == 5  # v1->v2, v2->v3, v3->v4, v4->v5, v5->v6

        # Prune to 2
        versions_dir = lp / "prune-test" / ".versions"
        pruned = mgr._prune_versions(versions_dir, "system.md", 2)
        assert pruned == 3
        remaining = mgr.get_versions(expert["id"], "system.md")
        assert len(remaining) == 2

    def test_prune_via_max_versions_config(self, temp_experts):
        """maxVersions in expert.json triggers auto-pruning on update."""
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Auto Prune", "coder", {"systemPrompt": "v1"})
        import time

        # Set maxVersions to 2
        ej_path = lp / "auto-prune" / "expert.json"
        data = json.loads(ej_path.read_text())
        data["maxVersions"] = 2
        ej_path.write_text(json.dumps(data, indent=2))

        # Create 5 updates — should auto-prune to 2 versions
        for i in range(2, 7):
            mgr.update_file(expert["id"], "system.md", f"v{i}")
            time.sleep(0.01)

        versions = mgr.get_versions(expert["id"], "system.md")
        assert len(versions) <= 2

    def test_prune_with_min_one(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Min Prune", "coder", {"systemPrompt": "v1"})
        import time

        mgr.update_file(expert["id"], "system.md", "v2")
        time.sleep(0.01)
        mgr.update_file(expert["id"], "system.md", "v3")

        versions_dir = lp / "min-prune" / ".versions"
        pruned = mgr._prune_versions(versions_dir, "system.md", 1)
        assert pruned == 1
        remaining = mgr.get_versions(expert["id"], "system.md")
        assert len(remaining) == 1

    def test_prune_invalid_max_treated_as_one(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Invalid Max", "coder", {"systemPrompt": "v1"})
        import time

        mgr.update_file(expert["id"], "system.md", "v2")
        time.sleep(0.01)

        versions_dir = lp / "invalid-max" / ".versions"
        pruned = mgr._prune_versions(versions_dir, "system.md", 0)
        assert pruned == 0  # 0 is treated as 1, but only 1 version exists
        remaining = mgr.get_versions(expert["id"], "system.md")
        assert len(remaining) == 1

    def test_prune_no_versions_is_noop(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("No Versions", "coder", {})
        versions_dir = lp / "no-versions" / ".versions"
        pruned = mgr._prune_versions(versions_dir, "system.md", 5)
        assert pruned == 0


class TestExpertManagerDelete:
    def test_delete_local_expert(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Delete Me", "coder", {})
        assert mgr.delete_expert(expert["id"]) is True
        assert not (lp / "delete-me").exists()

    def test_cannot_delete_marketplace(self, temp_experts):
        mgr, _, _ = temp_experts
        mgr.load_all()
        with pytest.raises(ValueError, match="marketplace"):
            mgr.delete_expert("marketplace-test-expert")

    def test_delete_nonexistent(self, temp_experts):
        mgr, _, _ = temp_experts
        assert mgr.delete_expert("nonexistent") is False

    def test_delete_removes_from_cache(self, temp_experts):
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Cache Delete", "coder", {})
        eid = expert["id"]
        assert mgr.get(eid) is not None
        mgr.delete_expert(eid)
        assert mgr._cache.get(eid) is None

    def test_delete_updates_registry(self, temp_experts):
        mgr, _, lp = temp_experts
        expert = mgr.create_local("Registry Delete", "coder", {})
        mgr.delete_expert(expert["id"])
        registry = json.loads((lp / "_registry.json").read_text())
        assert len(registry["experts"]) == 0


class TestExpertManagerEdgeCases:
    def test_load_corrupted_json(self, temp_experts):
        """Corrupted expert.json should not crash load_all."""
        mgr, mp, _ = temp_experts
        bad_dir = mp / "bad-expert"
        bad_dir.mkdir()
        (bad_dir / "expert.json").write_text("{invalid json")
        experts = mgr.load_all()
        # Should still load the valid expert
        assert any(e["name"] == "Test Expert" for e in experts)

    def test_load_missing_expert_json(self, temp_experts):
        """Directory without expert.json should be skipped."""
        mgr, mp, _ = temp_experts
        empty_dir = mp / "empty-dir"
        empty_dir.mkdir()
        experts = mgr.load_all()
        assert not any(e.get("_dir", "").endswith("empty-dir") for e in experts)

    def test_create_duplicate_name(self, temp_experts):
        """Creating two experts with same name should not crash."""
        mgr, _, _ = temp_experts
        e1 = mgr.create_local("Same Name", "coder", {})
        e2 = mgr.create_local("Same Name", "writer", {})
        # Second one overwrites or coexists
        assert e1["id"] == e2["id"]  # Same slug

    def test_update_nonexistent_expert(self, temp_experts):
        """Updating a nonexistent expert should raise."""
        mgr, _, _ = temp_experts
        with pytest.raises(ValueError):
            mgr.update_file("nonexistent-id", "system.md", "content")

    def test_get_versions_no_versions_dir(self, temp_experts):
        """Expert without .versions dir should return empty list."""
        mgr, _, _ = temp_experts
        mgr.load_all()
        versions = mgr.get_versions("marketplace-test-expert", "system.md")
        assert versions == []

    def test_restore_nonexistent_version(self, temp_experts):
        """Restoring a nonexistent version file should raise."""
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Restore Fail", "coder", {})
        with pytest.raises(ValueError):
            mgr.restore_version(expert["id"], "system.md.v9999999")

    def test_list_files_excludes_hidden(self, temp_experts):
        """Hidden files and .versions dir should not appear in file list."""
        mgr, _, _ = temp_experts
        expert = mgr.create_local("Hidden Test", "coder", {})
        files = mgr.list_files(expert["id"])
        names = [f["name"] for f in files]
        assert not any(n.startswith(".") for n in names)

    def test_get_prompt_missing_file(self, temp_experts):
        """Getting a prompt that doesn't exist should return empty string."""
        mgr, _, _ = temp_experts
        mgr.load_all()
        result = mgr.get_prompt("marketplace-test-expert", "nonexistent")
        assert result == ""
