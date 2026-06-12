"""Batch A content + model views — pure unit tests (no server)."""

from __future__ import annotations

from kortecx import ContentItem, ModelSummary, PutResult
from kortecx.v1 import gateway_pb2 as g


def test_put_result_from_proto_hex_encodes_the_server_derived_ref():
    r = g.PutContentResponse(content_ref=b"\xcd" * 32, size=1234, deduplicated=True)
    put = PutResult.from_proto(r)
    assert put.content_ref == "cd" * 32
    assert put.size == 1234
    assert put.deduplicated is True


def test_content_item_carries_payload_and_honest_truncation():
    i = g.ContentBatchItem(
        content_ref=b"\xee" * 32, payload=b"partial tex", truncated=True, full_size=999
    )
    item = ContentItem.from_proto(i)
    assert item.content_ref == "ee" * 32
    assert item.text == "partial tex"
    assert item.truncated is True
    assert item.full_size == 999
    assert item.missing is False


def test_content_item_uniform_empty_is_missing():
    # The uniform empty item (unauthorized / missing / malformed ref) — no
    # existence oracle (D120.1): empty payload AND zero full_size.
    i = g.ContentBatchItem(content_ref=b"\x11" * 32, payload=b"", truncated=False, full_size=0)
    assert ContentItem.from_proto(i).missing is True


def test_model_summary_from_proto_carries_display_fields():
    m = g.ModelSummary(
        model_id="kx-serve:qwen3-4b",
        modalities=["text", "image"],
        description="Qwen3 4B",
        serving=True,
        context_len=8192,
    )
    s = ModelSummary.from_proto(m)
    assert s.model_id == "kx-serve:qwen3-4b"
    assert s.modalities == ("text", "image")
    assert s.description == "Qwen3 4B"
    assert s.serving is True
    assert s.context_len == 8192
