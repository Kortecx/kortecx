"""POC-2 context-edit SDK helpers — pure-unit selector tests + a server-backed
edit/remove/export round trip over a real ``kx serve``.

The edit family is pure CLIENT composition over existing RPCs (GetContextBundle →
PutContent → PutContextBundle re-upsert): no proto/journal change, digest-invariant
by construction. These tests pin the selector semantics + the stale-base guard +
the empty-bundle refusal.
"""

from __future__ import annotations

import pytest

from kortecx import KxClient
from kortecx.context import ContextBundle, ContextBundleItem
from kortecx.errors import KxFailedPrecondition, KxUsage


def _bundle(*names: str) -> ContextBundle:
    items = [
        ContextBundleItem(name=n, content_ref=f"{i:02x}" * 32, media_type="text/plain")
        for i, n in enumerate(names)
    ]
    return ContextBundle(
        bundle_ref="ab" * 16, handle="t/ctx/b", description="", items=items, item_count=len(items)
    )


# ---- pure-unit selector semantics (no server) -------------------------------


def test_resolve_item_by_name_and_index():
    b = _bundle("intro", "body")
    assert KxClient._resolve_context_item(b, "body")[0] == 1
    assert KxClient._resolve_context_item(b, 0)[1].name == "intro"


def test_resolve_item_unknown_name_is_usage_error():
    with pytest.raises(KxUsage):
        KxClient._resolve_context_item(_bundle("intro"), "missing")


def test_resolve_item_ambiguous_name_requires_index():
    b = _bundle("dup", "dup")
    with pytest.raises(KxUsage, match="ambiguous"):
        KxClient._resolve_context_item(b, "dup")
    # The index disambiguates.
    assert KxClient._resolve_context_item(b, 1)[0] == 1


def test_resolve_item_out_of_range_index_is_usage_error():
    with pytest.raises(KxUsage, match="out of range"):
        KxClient._resolve_context_item(_bundle("intro"), 5)
    with pytest.raises(KxUsage):
        KxClient._resolve_context_item(_bundle("intro"), -1)


def test_resolve_item_rejects_bool_selector():
    # bool is an int subtype — True/False must NOT silently mean index 1/0.
    with pytest.raises(KxUsage):
        KxClient._resolve_context_item(_bundle("intro", "body"), True)


# ---- server-backed round trip ------------------------------------------------


def test_edit_remove_export_round_trip(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        a = kx.put_content(b"alpha", media_type="text/plain", filename="a.txt").content_ref
        b = kx.put_content(b"beta", media_type="text/plain", filename="b.txt").content_ref
        put = kx.put_context_bundle(
            "t/ctx/docs", [("a", a, "text/plain"), ("b", b, "text/plain")], description="d"
        )

        # export returns the FULL body (uploads scope, uncapped).
        assert kx.export_context_item("t/ctx/docs", "a") == b"alpha"
        assert kx.export_context_item("t/ctx/docs", 1) == b"beta"

        # edit re-points item "a" at NEW bytes; name + media preserved; description kept.
        res = kx.edit_context_item("t/ctx/docs", "a", b"ALPHA-v2")
        assert res.bundle_ref != put.bundle_ref  # the manifest changed
        assert kx.export_context_item("t/ctx/docs", "a") == b"ALPHA-v2"
        bundle = kx.get_context_bundle("t/ctx/docs")
        assert bundle is not None and bundle.description == "d"
        assert {it.name for it in bundle.items} == {"a", "b"}

        # editing back to identical bytes re-reports deduplicated at the content layer
        # (the manifest still re-upserts; the put is a dedup hit).
        again = kx.edit_context_item("t/ctx/docs", "a", b"ALPHA-v2")
        assert again.deduplicated is True

        # remove drops item "b" (re-upsert); export of "b" now errors as unknown.
        kx.remove_context_item("t/ctx/docs", "b")
        left = kx.get_context_bundle("t/ctx/docs")
        assert left is not None and [it.name for it in left.items] == ["a"]
        with pytest.raises(KxUsage):
            kx.export_context_item("t/ctx/docs", "b")

        # removing the last item is refused (use delete_context_bundle instead).
        with pytest.raises(KxUsage, match="empty"):
            kx.remove_context_item("t/ctx/docs", "a")


def test_stale_base_guard_fails_closed(dev_server):
    with KxClient(dev_server.endpoint) as kx:
        a = kx.put_content(b"v1", media_type="text/plain", filename="a.txt").content_ref
        put = kx.put_context_bundle("t/ctx/race", [("a", a, "text/plain")])
        stale = put.bundle_ref

        # A concurrent writer changes the bundle (a second item ⇒ a new bundle_ref).
        b = kx.put_content(b"added", media_type="text/plain", filename="b.txt").content_ref
        kx.put_context_bundle("t/ctx/race", [("a", a, "text/plain"), ("b", b, "text/plain")])

        # An edit holding the STALE bundle_ref is refused (no silent clobber).
        with pytest.raises(KxFailedPrecondition, match="changed since"):
            kx.edit_context_item("t/ctx/race", "a", b"v2", expect_bundle_ref=stale)

        # The same edit with the CURRENT ref (or none) succeeds.
        current = kx.get_context_bundle("t/ctx/race")
        assert current is not None
        kx.edit_context_item("t/ctx/race", "a", b"v2", expect_bundle_ref=current.bundle_ref)
        assert kx.export_context_item("t/ctx/race", "a") == b"v2"
