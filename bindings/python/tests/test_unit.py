"""Pure unit tests — no server. Hex, errors, type views, args encoding, Result."""

from __future__ import annotations

import grpc
import pytest

from kortecx import (
    ErrorCode,
    KxCatchupRequired,
    KxPermissionDenied,
    KxRunFailed,
    KxUnauthenticated,
    KxUsage,
    KxWaitTimeout,
    Result,
    WaitState,
    hexids,
)
from kortecx.client import _encode_args, _is_nonloopback_plaintext, _resolve_token, _target
from kortecx.errors import from_rpc_error
from kortecx.run import Result as RunResult
from kortecx.types import Delta, MoteView, Projection, is_committed, is_pending, state_name
from kortecx.v1 import gateway_pb2 as g
from kortecx.wait import WaitOutcome

# --- hex (SN-8 safe: only encode/decode, never derive) ------------------------


def test_hex_roundtrip_and_lengths():
    assert hexids.encode(b"\x00\xab") == "00ab"
    assert hexids.decode("00AB") == b"\x00\xab"
    assert hexids.instance_id("ab" * 16) == b"\xab" * 16
    assert hexids.ref32("cd" * 32) == b"\xcd" * 32


def test_hex_rejects_bad_input():
    with pytest.raises(KxUsage):
        hexids.decode("zz")
    with pytest.raises(KxUsage):
        hexids.instance_id("ab" * 8)  # 8 bytes, not 16
    with pytest.raises(KxUsage):
        hexids.ref32("cd" * 16)  # 16 bytes, not 32


def test_as_bytes_accepts_hex_or_bytes():
    assert hexids.as_bytes("ab" * 16, 16) == b"\xab" * 16
    assert hexids.as_bytes(b"\x01" * 32, 32) == b"\x01" * 32
    with pytest.raises(KxUsage):
        hexids.as_bytes(b"\x01" * 4, 32)


# --- errors -------------------------------------------------------------------


class _FakeRpc(grpc.RpcError):
    def __init__(self, code, details):
        self._code = code
        self._details = details

    def code(self):
        return self._code

    def details(self):
        return self._details


@pytest.mark.parametrize(
    "status,cls,code",
    [
        (grpc.StatusCode.UNAUTHENTICATED, KxUnauthenticated, ErrorCode.UNAUTHENTICATED),
        (grpc.StatusCode.PERMISSION_DENIED, KxPermissionDenied, ErrorCode.PERMISSION_DENIED),
        (grpc.StatusCode.RESOURCE_EXHAUSTED, KxCatchupRequired, ErrorCode.CATCHUP_REQUIRED),
    ],
)
def test_from_rpc_error_maps_status(status, cls, code):
    err = from_rpc_error(_FakeRpc(status, "boom"))
    assert isinstance(err, cls)
    assert err.code == code
    assert "boom" in str(err)
    assert err.grpc_code == status.name


def test_error_carry_fields():
    assert KxCatchupRequired("x", next_seq=7).next_seq == 7
    e = KxWaitTimeout("t", instance_id="aa", terminal_mote_id="bb")
    assert e.instance_id == "aa" and e.terminal_mote_id == "bb"
    f = KxRunFailed("f", instance_id="aa", terminal_mote_id="bb")
    assert f.code == ErrorCode.RUN_FAILED


# --- type views ---------------------------------------------------------------


def test_state_name_and_predicates():
    assert state_name(g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED) == "COMMITTED"
    assert state_name(999) == "UNKNOWN"
    assert is_committed(g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED)
    assert is_pending(g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_SCHEDULED)
    assert not is_pending(g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED)


def test_mote_and_projection_views():
    snap = g.MoteSnapshot(
        mote_id=b"\x03" * 32,
        state=g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_COMMITTED,
        nd_class=1,
        promotion=1,
        result_ref=b"\x04" * 32,
        mote_def_hash=b"\x05" * 32,
        committed_seq=7,
    )
    view = g.ProjectionView(
        instance_id=b"\x01" * 16, recipe_fingerprint=b"\x02" * 32, current_seq=7, motes=[snap]
    )
    proj = Projection.from_proto(view)
    assert proj.current_seq == 7
    assert proj.motes[0].state == "COMMITTED"
    assert proj.motes[0].result_ref == "04" * 32
    assert proj.committed and proj.mote("03" * 32) is not None
    d = proj.to_dict()
    assert d["motes"][0]["state"] == "COMMITTED" and d["motes"][0]["committed_seq"] == 7


def test_mote_view_optional_result_ref_absent():
    snap = g.MoteSnapshot(
        mote_id=b"\x03" * 32,
        state=g.MoteSnapshotState.MOTE_SNAPSHOT_STATE_SCHEDULED,
        nd_class=1,
        promotion=1,
        mote_def_hash=b"\x05" * 32,
    )
    mv = MoteView.from_proto(snap)
    assert mv.result_ref is None and mv.committed_seq is None


def test_delta_views_cover_oneof():
    committed = g.EventDelta(
        seq=5, committed=g.CommittedDelta(mote_id=b"\x07" * 32, result_ref=b"\x08" * 32, nd_class=1)
    )
    dv = Delta.from_proto(committed)
    assert dv.kind == "committed" and dv.mote_id == "07" * 32 and dv.to_dict()["seq"] == 5
    failed = g.EventDelta(seq=6, failed=g.FailedDelta(mote_id=b"\x09" * 32, reason_class=3))
    assert Delta.from_proto(failed).reason_class == 3
    assert Delta.from_proto(g.EventDelta(seq=1)) is None  # no kind → skipped


# --- args encoding + credential resolution ------------------------------------


def test_encode_args_variants():
    assert _encode_args({"topic": "x"}) == b'{"topic":"x"}'
    assert _encode_args('{"a":1}') == b'{"a":1}'
    assert _encode_args(b'{"a":1}') == b'{"a":1}'
    with pytest.raises(KxUsage):
        _encode_args("{not json")
    with pytest.raises(KxUsage):
        _encode_args(123)  # type: ignore[arg-type]


def test_plaintext_detection():
    assert _is_nonloopback_plaintext("http://example.com:50151")
    assert _is_nonloopback_plaintext("http://10.0.0.5:50151")
    assert not _is_nonloopback_plaintext("http://127.0.0.1:50151")
    assert not _is_nonloopback_plaintext("http://localhost:50151")
    assert not _is_nonloopback_plaintext("http://[::1]:50151")
    assert not _is_nonloopback_plaintext("https://example.com")


def test_target_strips_scheme():
    assert _target("http://127.0.0.1:50151") == "127.0.0.1:50151"
    assert _target("https://h:1/") == "h:1"
    assert _target("h:1") == "h:1"


def test_resolve_token_precedence(tmp_path, monkeypatch):
    monkeypatch.delenv("KX_TOKEN", raising=False)
    assert _resolve_token("http://127.0.0.1:1", None, None) is None
    # token_file is read + trimmed
    f = tmp_path / "tok"
    f.write_text("  s3cr3t\n")
    assert _resolve_token("http://127.0.0.1:1", None, str(f)) == "s3cr3t"
    # mutually exclusive
    with pytest.raises(KxUsage):
        _resolve_token("http://127.0.0.1:1", "t", str(f))
    # empty file is a usage error
    empty = tmp_path / "empty"
    empty.write_text("  \n")
    with pytest.raises(KxUsage):
        _resolve_token("http://127.0.0.1:1", None, str(empty))
    # env fallback
    monkeypatch.setenv("KX_TOKEN", "envtok")
    assert _resolve_token("http://127.0.0.1:1", None, None) == "envtok"


def test_plaintext_token_warns():
    with pytest.warns(UserWarning):
        _resolve_token("http://example.com:50151", "t", None)


# --- Result shape (parity with the CLI render_wait) ---------------------------


def test_result_committed_to_dict():
    out = WaitOutcome(
        instance_id=b"\x01" * 16,
        terminal_mote_id=b"\x02" * 32,
        state=WaitState.COMMITTED,
        result_ref=b"\x03" * 32,
        payload=b"hello",
    )
    r = RunResult.from_outcome(out)
    assert r.ok and r.text == "hello" and r.bytes == b"hello"
    d = r.to_dict()
    assert d["state"] == "COMMITTED"
    assert d["result_utf8"] == "hello" and d["result_len"] == 5
    assert d["result_hex"] == b"hello".hex()
    # --out path omits the payload bytes but keeps the length
    meta = r.to_dict(include_payload=False)
    assert "result_hex" not in meta and meta["result_len"] == 5


def test_result_running_flags_timeout():
    out = WaitOutcome(
        instance_id=b"\x01" * 16, terminal_mote_id=b"\x02" * 32, state=WaitState.RUNNING
    )
    r = Result.from_outcome(out)
    assert r.timed_out and r.to_dict()["timed_out"] is True


def test_result_binary_payload_has_no_utf8():
    out = WaitOutcome(
        instance_id=b"\x01" * 16,
        terminal_mote_id=b"\x02" * 32,
        state=WaitState.COMMITTED,
        result_ref=b"\x03" * 32,
        payload=b"\xff\xfe\x00",
    )
    r = Result.from_outcome(out)
    assert r.text is None and "result_utf8" not in r.to_dict()
