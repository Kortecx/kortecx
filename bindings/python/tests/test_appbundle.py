"""``kortecx.appbundle/v1`` codec — cross-surface golden parity + structure tests.

The parity gate (GR12): every committed bundle in ``tests/golden/apps/bundle_corpus.json``
round-trips through this SDK's codec BYTE-IDENTICALLY (matches the Rust ``kx-appbundle``
crate + the TS SDK). ``content_refs`` mirrors the Rust envelope walk.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import kortecx as kx
from kortecx.appbundle import BUNDLE_SCHEMA, AppBundle
from kortecx.apps import content_refs
from kortecx.client import KxClient

_BUNDLE_CORPUS = (
    Path(__file__).resolve().parents[3] / "tests" / "golden" / "apps" / "bundle_corpus.json"
)


def _cases():
    return json.loads(_BUNDLE_CORPUS.read_text(encoding="utf-8"))


@pytest.mark.parametrize("case", _cases(), ids=lambda c: c["name"])
def test_bundle_corpus_round_trips_byte_identically(case) -> None:
    # from_json -> to_json must equal the committed bytes (idempotent canonicalization).
    parsed = AppBundle.from_json(case["bundle"])
    assert parsed.to_json() == case["bundle"], f"case {case['name']}: not byte-identical"


def test_from_json_rejects_bad_schema() -> None:
    with pytest.raises(ValueError):
        AppBundle.from_json(json.dumps({"app_digest": "a" * 64, "envelope": "{}", "schema": "x"}))


def test_from_json_rejects_bad_hex_ref() -> None:
    with pytest.raises(ValueError):
        AppBundle.from_json(
            json.dumps({"app_digest": "NOPE", "envelope": "{}", "schema": BUNDLE_SCHEMA})
        )


def test_empty_closure_omits_blobs_and_source_digest() -> None:
    bundle = AppBundle(app_digest="ab" * 32, envelope=b"{}")
    wire = bundle.to_json()
    assert "blobs" not in wire
    assert "source_digest" not in wire
    assert AppBundle.from_json(wire) == bundle


def test_round_trips_with_binary_blob_and_lineage() -> None:
    bundle = AppBundle(
        app_digest="11" * 32,
        envelope=b'{"name":"x","schema":"kortecx.app/v1"}',
        blobs={"aa" * 32: bytes([0, 1, 2, 253, 254, 255])},
        source_digest="22" * 32,
    )
    parsed = AppBundle.from_json(bundle.to_json())
    assert parsed == bundle
    assert parsed.to_json() == bundle.to_json()


def test_content_refs_walks_sorts_and_gates_datasets() -> None:
    envelope = {
        "references": {
            "prompts": [{"name": "p", "content_ref": "aa" * 32}],
            "rules": [{"name": "r", "content_ref": "bb" * 32}],
            "skills": [{"name": "s", "instructions_ref": "cc" * 32}],
            "datasets": [{"dataset_ref": "d", "cas_refs": ["dd" * 32]}],
        },
        "steering_config": {"context": {"context_refs": ["ee" * 32]}},
    }
    without = content_refs(envelope)
    assert without == ["aa" * 32, "bb" * 32, "cc" * 32, "ee" * 32]
    with_data = content_refs(envelope, include_datasets=True)
    assert "dd" * 32 in with_data
    assert len(with_data) == 5


# ---- server-backed (a real kx serve): export → import → clone round-trip ----


def test_export_import_clone_round_trip(dev_server) -> None:
    with KxClient(dev_server.endpoint) as client:
        # An App with a rule body → a content-store blob that must travel in the bundle.
        a = (
            kx.app("Bundle Demo")
            .blueprint(kx.flow().step(topic="hi"))
            .rule("cite", body="Always cite your sources.")
        )
        saved = a.save(client=client)
        handle = saved.handle
        original = client.get_app(handle)
        assert original is not None
        assert len(original.app_digest) == 64
        assert original.source_digest == ""  # authored here

        # EXPORT: a bundle carrying the rule blob, named by the App's app_digest.
        wire = client.export_app_bundle(handle)
        bundle = AppBundle.from_json(wire)
        assert bundle.app_digest == original.app_digest
        assert bundle.blob_count() == 1

        # IMPORT (force, over the same handle): the app_digest round-trips identically
        # (SN-4 determinism) and the source lineage is stamped.
        client.import_app(wire, force=True)
        reimported = client.get_app(handle)
        assert reimported is not None
        assert reimported.app_digest == original.app_digest
        assert reimported.source_digest == original.app_digest

        # CLONE: a new App under a new name, with lineage back to the source.
        cloned = client.clone_app(handle, "Bundle Copy")
        assert cloned.handle == "apps/local/bundle-copy"
        copy = client.get_app("apps/local/bundle-copy")
        assert copy is not None
        assert copy.envelope["name"] == "Bundle Copy"
        assert copy.source_digest == original.app_digest
        # Renaming changes the canonical envelope ⇒ a DIFFERENT app_digest.
        assert copy.app_digest != original.app_digest
