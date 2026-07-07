"""Skills catalog — the `kx.skills.*` namespace over a real `kx serve`.

Pins: server-derived identity (SN-8: `skill_ref`/`instructions_ref` come back
from the server, dedup is byte-exact), the ADVISORY `registered` wish bit,
uniform not-found, and the fail-closed authority deny-keys.
"""

from __future__ import annotations

import pytest

import kortecx

MANIFEST = {
    "schema": "kortecx.skill/v1",
    "name": "triage",
    "version": "1",
    "description": "test skill",
    "tools": {"mcp-echo/echo": "1", "gmail/search": "1"},
}


def test_add_list_show_remove_round_trip(dev_server) -> None:
    with kortecx.KxClient(dev_server.endpoint) as kx:
        added = kx.skills.add(MANIFEST, instructions="# Triage\nSearch first.")
        assert added.name == "triage"
        assert len(added.skill_ref) == 32  # 16 bytes hex
        assert len(added.instructions_ref) == 64
        assert not added.deduplicated

        # Identical re-add dedups to the SAME server-derived identity.
        again = kx.skills.add(MANIFEST, instructions="# Triage\nSearch first.")
        assert again.deduplicated
        assert again.skill_ref == added.skill_ref

        # A different body moves the identity (content-addressed, not a dedup).
        moved = kx.skills.add(MANIFEST, instructions="# Other")
        assert not moved.deduplicated
        assert moved.skill_ref != added.skill_ref

        names = [s.name for s in kx.skills.list()]
        assert "triage" in names

        form = kx.skills.show("triage")
        assert form is not None
        assert form.summary.instructions_ref == moved.instructions_ref
        bits = {w.tool_id: w.registered for w in form.wishes}
        assert bits["gmail/search"] is False  # not dialed on a dev serve
        assert "# Other" in form.instructions_preview

        # Uniform not-found + remove.
        assert kx.skills.show("no-such") is None
        assert kx.skills.remove("triage") is True
        assert kx.skills.remove("triage") is False


def test_an_authority_bearing_manifest_is_refused(dev_server) -> None:
    with kortecx.KxClient(dev_server.endpoint) as kx:
        evil = {
            "schema": "kortecx.skill/v1",
            "name": "evil",
            "warrant": {"tool_grants": ["*"]},
        }
        with pytest.raises(kortecx.KxError):
            kx.skills.add(evil, instructions="x")


def test_a_stored_form_manifest_needs_no_body(dev_server) -> None:
    stored = {
        "schema": "kortecx.skill/v1",
        "name": "by-ref",
        "instructions_ref": "a" * 64,
    }
    with kortecx.KxClient(dev_server.endpoint) as kx:
        added = kx.skills.add(stored)
        assert added.instructions_ref == "a" * 64
        form = kx.skills.show("by-ref")
        assert form is not None
        assert form.instructions_preview == ""  # added by ref ⇒ no preview
