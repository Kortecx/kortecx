"""Batch C monitoring views — pure unit tests, no server: the global event tail
(gRPC oneof + WS JSON parsing), the WS URL shapes, and the telemetry mappers."""

from __future__ import annotations

from kortecx import GlobalDelta, MoteTelemetryRow, TelemetryPage
from kortecx.events import _ws_all_url, _ws_global_delta, _ws_url
from kortecx.telemetry import MoteTelemetryRow as TelemetryRowAlias
from kortecx.v1 import gateway_pb2 as g

# --- WS URL formatting --------------------------------------------------------


def test_ws_all_url_formats_without_instance_param():
    # Derived from the gRPC endpoint: scheme/host mapped to the WS port (50152).
    assert (
        _ws_all_url("http://127.0.0.1:50151", None, 7)
        == "ws://127.0.0.1:50152/v1/events/all?since=7"
    )
    assert (
        _ws_all_url("https://gw.example.com:443", None, 0)
        == "wss://gw.example.com:50152/v1/events/all?since=0"
    )
    # An explicit ws endpoint wins (trailing slash trimmed).
    assert _ws_all_url("http://ignored", "ws://h:9/", 3) == "ws://h:9/v1/events/all?since=3"


def test_ws_per_run_url_still_formats():
    # Regression for the shared base extraction: the per-run URL is unchanged.
    assert (
        _ws_url("http://127.0.0.1:50151", None, "ab" * 16, 5)
        == f"ws://127.0.0.1:50152/v1/events?instance={'ab' * 16}&since=5"
    )


# --- the global delta view (gRPC oneof) ----------------------------------------


def test_global_delta_from_proto_covers_every_kind():
    rr = g.GlobalEventDelta(
        seq=1,
        instance_id=b"\x01" * 16,
        run_registered=g.RunRegisteredDelta(
            recipe_fingerprint=b"\x2a" * 32, registered_unix_ms=123
        ),
    )
    dv = GlobalDelta.from_proto(rr)
    assert dv.kind == "run_registered"
    assert dv.instance_id == "01" * 16
    assert dv.recipe_fingerprint == "2a" * 32
    assert dv.registered_unix_ms == 123

    committed = g.GlobalEventDelta(
        seq=2,
        instance_id=b"\x01" * 16,
        committed=g.CommittedDelta(mote_id=b"\x07" * 32, result_ref=b"\x08" * 32, nd_class=1),
    )
    dv = GlobalDelta.from_proto(committed)
    assert dv.kind == "committed" and dv.mote_id == "07" * 32 and dv.nd_class == 1

    failed = g.GlobalEventDelta(
        seq=3, instance_id=b"\x01" * 16, failed=g.FailedDelta(mote_id=b"\x09" * 32, reason_class=3)
    )
    assert GlobalDelta.from_proto(failed).reason_class == 3

    repudiated = g.GlobalEventDelta(
        seq=4,
        instance_id=b"\x01" * 16,
        repudiated=g.RepudiatedDelta(target_mote_id=b"\x0a" * 32, target_committed_seq=2),
    )
    dv = GlobalDelta.from_proto(repudiated)
    assert dv.kind == "repudiated" and dv.target_committed_seq == 2

    staged = g.GlobalEventDelta(
        seq=5, instance_id=b"\x01" * 16, effect_staged=g.EffectStagedDelta(mote_id=b"\x0b" * 32)
    )
    assert GlobalDelta.from_proto(staged).kind == "effect_staged"


def test_global_delta_empty_instance_pre_registration():
    # EMPTY before any registration (the watermark has nothing to stamp) → "".
    d = g.GlobalEventDelta(
        seq=1,
        instance_id=b"",
        committed=g.CommittedDelta(mote_id=b"\x07" * 32, result_ref=b"\x08" * 32, nd_class=1),
    )
    assert GlobalDelta.from_proto(d).instance_id == ""


def test_global_delta_no_kind_surfaces_as_unknown():
    # The global-tail contract (TS/CLI parity): a future delta kind SURFACES as
    # "unknown" — never silently dropped (unlike the per-run Delta skip).
    view = GlobalDelta.from_proto(g.GlobalEventDelta(seq=1, instance_id=b"\x01" * 16))
    assert view.kind == "unknown"
    assert view.seq == 1
    assert view.instance_id == "01" * 16


def test_global_delta_to_dict_keeps_relevant_fields_only():
    d = GlobalDelta(seq=9, kind="failed", instance_id="01" * 16, mote_id="07" * 32, reason_class=2)
    out = d.to_dict()
    assert out == {
        "seq": 9,
        "kind": "failed",
        "instance_id": "01" * 16,
        "mote_id": "07" * 32,
        "reason_class": 2,
    }


# --- the global WS JSON parser --------------------------------------------------


def test_ws_global_delta_parses_each_type():
    rr = _ws_global_delta(
        {
            "type": "run_registered",
            "seq": 1,
            "instance_id": "01" * 16,
            "recipe_fingerprint": "2a" * 32,
            "registered_unix_ms": 123,
        }
    )
    assert rr.kind == "run_registered" and rr.recipe_fingerprint == "2a" * 32
    assert rr.registered_unix_ms == 123

    c = _ws_global_delta(
        {
            "type": "committed",
            "seq": 2,
            "instance_id": "01" * 16,
            "mote_id": "07" * 32,
            "result_ref": "08" * 32,
            "nd_class": "pure",
        }
    )
    assert c.kind == "committed" and c.mote_id == "07" * 32 and c.nd_class == "pure"

    f = _ws_global_delta(
        {
            "type": "failed",
            "seq": 3,
            "instance_id": "01" * 16,
            "mote_id": "09" * 32,
            "reason_class": 3,
        }
    )
    assert f.kind == "failed" and f.reason_class == 3

    r = _ws_global_delta(
        {
            "type": "repudiated",
            "seq": 4,
            "instance_id": "01" * 16,
            "target_mote_id": "0a" * 32,
            "target_committed_seq": 2,
        }
    )
    assert r.kind == "repudiated" and r.target_committed_seq == 2

    e = _ws_global_delta(
        {"type": "effect_staged", "seq": 5, "instance_id": "01" * 16, "mote_id": "0b" * 32}
    )
    assert e.kind == "effect_staged" and e.mote_id == "0b" * 32


def test_ws_global_delta_empty_instance_and_unknown_tolerance():
    # "" instance (pre-registration) is honest, not an error.
    d = _ws_global_delta(
        {
            "type": "committed",
            "seq": 1,
            "instance_id": "",
            "mote_id": "07" * 32,
            "result_ref": "08" * 32,
            "nd_class": "pure",
        }
    )
    assert d.instance_id == ""
    # The server's explicit `unknown` variant AND any future tag SURFACE as
    # "unknown" (never dropped — TS/CLI parity on the global tail).
    assert _ws_global_delta({"type": "unknown", "seq": 9, "instance_id": ""}).kind == "unknown"
    future = _ws_global_delta({"type": "telemetry_flushed", "seq": 10})
    assert future.kind == "unknown"
    assert future.seq == 10
    assert _ws_global_delta({}).kind == "unknown"


# --- telemetry mappers ----------------------------------------------------------


def test_telemetry_row_maps_every_field():
    row = g.MoteTelemetryRow(
        mote_id=b"\xa1" * 32,
        instance_id=b"\x01" * 16,
        wall_clock_ms=42,
        output_tokens=17,
        model_id="kx-serve:qwen3-4b",
        tool_id="mcp-echo@1",
        started_unix_ms=1_700_000_000_000,
        seq=9,
    )
    view = MoteTelemetryRow.from_proto(row)
    assert view.mote_id == "a1" * 32
    assert view.instance_id == "01" * 16
    assert view.wall_clock_ms == 42
    assert view.input_tokens is None  # never set in OSS
    assert view.output_tokens == 17
    assert view.model_id == "kx-serve:qwen3-4b"
    assert view.tool_id == "mcp-echo@1"
    assert view.started_unix_ms == 1_700_000_000_000
    assert view.seq == 9


def test_telemetry_row_absent_optionals_are_none_not_zero():
    # optional uint64: absent ≠ 0 — a non-model mote has None, not a 0 count.
    row = g.MoteTelemetryRow(mote_id=b"\xa1" * 32, instance_id=b"\x00" * 16, wall_clock_ms=1, seq=2)
    view = MoteTelemetryRow.from_proto(row)
    assert view.input_tokens is None and view.output_tokens is None
    assert view.model_id == "" and view.tool_id == ""
    # All-zero instance = unattributed; hex-encoded verbatim (the capture-row
    # convention — the SDK never invents a sentinel).
    assert view.instance_id == "00" * 16


def test_telemetry_page_shape():
    row = MoteTelemetryRow.from_proto(
        g.MoteTelemetryRow(mote_id=b"\xa1" * 32, instance_id=b"\x01" * 16, wall_clock_ms=1, seq=1)
    )
    page = TelemetryPage(rows=[row], has_more=True)
    assert page.rows[0] is row and page.has_more is True
    assert TelemetryRowAlias is MoteTelemetryRow  # one class, exported at the top level
