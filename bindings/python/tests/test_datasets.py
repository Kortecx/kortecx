"""T3.7 Datasets data-plane views — pure unit tests (no server)."""

from __future__ import annotations

from kortecx import DatasetHit, DatasetSummary, FuzzyHit, IngestDocument, IngestResult
from kortecx.v1 import gateway_pb2 as g


def test_dataset_summary_from_proto_carries_counts():
    d = g.DatasetSummary(dataset_id="corpus", name="corpus", doc_count=42, dim=64, created_ms=1234)
    s = DatasetSummary.from_proto(d)
    assert s.dataset_id == "corpus"
    assert s.doc_count == 42
    assert s.dim == 64
    assert s.created_ms == 1234


def test_dataset_hit_from_proto_hex_encodes_ref_and_decodes_text():
    h = g.DatasetHit(content_ref=b"\xab" * 32, content=b"hello world", score=0.875)
    hit = DatasetHit.from_proto(h)
    assert hit.content_ref == "ab" * 32
    assert hit.content == b"hello world"
    assert abs(hit.score - 0.875) < 1e-6
    assert hit.text == "hello world"


def test_fuzzy_hit_from_proto_hex_encodes_ref_and_exposes_score_fraction():
    # Slice-B exact-out: refs + a DISPLAY-ONLY basis-point score (no content bytes).
    h = g.FuzzyHit(content_ref=b"\xcd" * 32, score_bp=8750)
    hit = FuzzyHit.from_proto(h)
    assert hit.content_ref == "cd" * 32
    assert hit.score_bp == 8750
    assert abs(hit.score - 0.875) < 1e-6  # bp/10000 fraction (display only)


def test_ingest_result_from_proto_carries_counts():
    r = g.IngestDocumentsResponse(dataset_id="corpus", doc_count=10, inserted=3, dim=64)
    res = IngestResult.from_proto(r)
    assert res.dataset_id == "corpus"
    assert res.doc_count == 10
    assert res.inserted == 3
    assert res.dim == 64


def test_ingest_document_to_proto_round_trips_fields():
    doc = IngestDocument(
        content=b"doc bytes",
        # Powers of two are exactly representable in proto's float32 (no rounding).
        embedding=[0.5, 0.25, 0.125],
        doc_id="advisory-id",
        metadata={"src": "unit"},
    )
    msg = doc.to_proto()
    assert msg.content == b"doc bytes"
    assert list(msg.embedding) == [0.5, 0.25, 0.125]
    assert msg.doc_id == "advisory-id"
    assert dict(msg.metadata) == {"src": "unit"}


def test_ingest_document_minimal_has_no_embedding():
    msg = IngestDocument(content=b"text only").to_proto()
    assert msg.content == b"text only"
    assert list(msg.embedding) == []
    assert msg.doc_id == ""
