"""W1.A5 toolscout views — pure unit tests (no server)."""

from __future__ import annotations

from kortecx import (
    BundleScore,
    BundleSpec,
    BundleTool,
    KeywordSet,
    ManifestScore,
    ToolManifest,
    lower_verdict_name,
)
from kortecx.v1 import gateway_pb2 as g


def test_keyword_set_round_trips():
    ks = KeywordSet.from_proto(g.KeywordSet(lang="en", words=["echo", "ping"]))
    assert ks.lang == "en"
    assert ks.words == ["echo", "ping"]
    msg = ks.to_proto()
    assert msg.lang == "en"
    assert list(msg.words) == ["echo", "ping"]


def test_tool_manifest_from_proto_hex_encodes_fingerprint():
    m = g.ToolManifest(
        tool_id="mcp-echo",
        tool_version="1",
        description="echoes its input",
        keywords=[
            g.KeywordSet(lang="en", words=["echo"]),
            g.KeywordSet(lang="ja", words=["エコー"]),
        ],
        fingerprint_hash=b"\xab" * 32,
        kind="Builtin",
    )
    tm = ToolManifest.from_proto(m)
    assert tm.tool_id == "mcp-echo"
    assert tm.tool_version == "1"
    assert tm.description == "echoes its input"
    assert [k.lang for k in tm.keywords] == ["en", "ja"]
    assert tm.keywords[0].words == ["echo"]
    assert tm.fingerprint_hash == "ab" * 32  # lowercase, 64 hex chars
    assert len(tm.fingerprint_hash) == 64
    assert tm.kind == "Builtin"


def test_manifest_score_from_proto_carries_bp_and_hex_hash():
    s = ManifestScore.from_proto(
        g.ManifestScore(
            tool_id="mcp-echo", tool_version="1", score_bp=9000, fingerprint_hash=b"\xcd" * 32
        )
    )
    assert s.tool_id == "mcp-echo"
    assert s.tool_version == "1"
    assert s.score_bp == 9000
    assert s.fingerprint_hash == "cd" * 32


def test_lower_verdict_name_maps_every_arm_and_unknown():
    assert lower_verdict_name(g.LowerVerdict.LOWER_VERDICT_UNAVAILABLE) == "unavailable"
    assert lower_verdict_name(g.LowerVerdict.LOWER_VERDICT_WOULD_LOWER) == "would-lower"
    assert lower_verdict_name(g.LowerVerdict.LOWER_VERDICT_REFUSED) == "refused"
    # "unknown" absorbs UNSPECIFIED(0) + any future value (the TS SDK mirror) —
    # never a crash.
    assert lower_verdict_name(g.LowerVerdict.LOWER_VERDICT_UNSPECIFIED) == "unknown"
    assert lower_verdict_name(99) == "unknown"


def test_bundle_score_from_proto_maps_verdict_and_ranks():
    r = g.ScoreTaskBundleResponse(
        bundle_fingerprint=b"\x12" * 32,
        ranked=[
            g.ManifestScore(
                tool_id="mcp-echo", tool_version="1", score_bp=10000, fingerprint_hash=b"\xab" * 32
            ),
            g.ManifestScore(
                tool_id="other", tool_version="2", score_bp=1500, fingerprint_hash=b"\xcd" * 32
            ),
        ],
        verdict=g.LowerVerdict.LOWER_VERDICT_WOULD_LOWER,
        verdict_detail="grant gate passed",
    )
    score = BundleScore.from_proto(r)
    assert score.bundle_fingerprint == "12" * 32
    assert [s.tool_id for s in score.ranked] == ["mcp-echo", "other"]
    assert score.ranked[0].score_bp == 10000
    assert score.verdict == "would-lower"
    assert score.verdict_detail == "grant gate passed"


def test_bundle_score_refused_and_unavailable_verdicts():
    refused = BundleScore.from_proto(
        g.ScoreTaskBundleResponse(
            bundle_fingerprint=b"\x00" * 32,
            verdict=g.LowerVerdict.LOWER_VERDICT_REFUSED,
            verdict_detail="duplicate tool name",
        )
    )
    assert refused.verdict == "refused"
    assert refused.ranked == []
    unavailable = BundleScore.from_proto(
        g.ScoreTaskBundleResponse(
            bundle_fingerprint=b"\x00" * 32,
            verdict=g.LowerVerdict.LOWER_VERDICT_UNAVAILABLE,
            verdict_detail="no live react runtime",
        )
    )
    assert unavailable.verdict == "unavailable"


def test_bundle_spec_to_proto_round_trips_fields():
    spec = BundleSpec(
        intent="echo the topic back",
        tools=[
            BundleTool(
                tool_id="mcp-echo",
                tool_version="1",
                description="echoes",
                keywords=[KeywordSet(lang="en", words=["echo"])],
            ),
            BundleTool(tool_id="other", tool_version="2"),
        ],
        language_tags=["en", "hi"],
        tolerance_threshold_bp=2500,
    )
    req = spec.to_proto()
    assert req.intent == "echo the topic back"
    assert list(req.language_tags) == ["en", "hi"]
    assert [t.tool_id for t in req.tool_sequence] == ["mcp-echo", "other"]
    assert req.tool_sequence[0].description == "echoes"
    assert req.tool_sequence[0].keywords[0].lang == "en"
    assert list(req.tool_sequence[0].keywords[0].words) == ["echo"]
    assert req.tolerance_threshold_bp == 2500


def test_bundle_spec_minimal_defaults():
    req = BundleSpec(intent="x", tools=[BundleTool(tool_id="t", tool_version="1")]).to_proto()
    assert req.intent == "x"
    assert list(req.language_tags) == []
    assert req.tolerance_threshold_bp == 0
    t = req.tool_sequence[0]
    assert t.description == ""
    assert list(t.keywords) == []
