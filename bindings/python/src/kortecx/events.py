"""Event-stream consumers.

Mirrors the CLI ``events`` verb: ``stream_deltas`` without ``follow`` reads one
snapshot (``since`` → the current journal boundary) and stops; with ``follow`` it
consumes the server's live tail and transparently reconnects from the last cursor
on a ``CatchupRequired`` (``RESOURCE_EXHAUSTED``) drop — no delta lost or
duplicated. Sync and async generators are provided.

An optional WebSocket client (`pip install 'kortecx[ws]'`) consumes the same live
tail over the R5 browser/firewall-friendly JSON bridge.

The Batch C GLOBAL twins (``stream_all_deltas`` / ``ws_stream_all_deltas``)
consume the cross-run tail (``StreamAllEvents`` / the WS ``/v1/events/all``
channel): every run on the node over one stream, each delta stamped with its
registration-watermark ``instance_id`` plus the ``run_registered`` kind the
per-run cursor never surfaces.
"""

from __future__ import annotations

from typing import AsyncIterator, Iterator, Optional

import grpc

from . import types
from .errors import from_rpc_error
from .v1 import gateway_pb2 as _g

# --- sync gRPC ---------------------------------------------------------------


def stream_deltas(stub, md, instance_id: bytes, since: int, follow: bool) -> Iterator[types.Delta]:
    """Yield a run's event deltas (one snapshot, or the live tail with ``follow``)."""
    cursor = since
    while True:
        req = _g.StreamEventsRequest(instance_id=instance_id, since_seq=cursor)
        try:
            for frame in stub.StreamEvents(req, metadata=md):
                for d in frame.deltas:
                    view = types.Delta.from_proto(d)
                    if view is not None:
                        yield view
                cursor = frame.next_seq
                if not follow and frame.journal_boundary:
                    return
        except grpc.RpcError as e:
            if follow and e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue  # CatchupRequired: resume from the last cursor
            raise from_rpc_error(e) from e
        if not follow:
            return


def stream_all_deltas(stub, md, since: int, follow: bool) -> Iterator[types.GlobalDelta]:
    """Yield the cross-run GLOBAL event tail (one snapshot, or the live tail with
    ``follow``) — Batch C ``StreamAllEvents``. Same cursor semantics as the
    per-run stream, incl. the ``RESOURCE_EXHAUSTED`` resume on a slow-consumer
    drop."""
    cursor = since
    while True:
        req = _g.StreamAllEventsRequest(since_seq=cursor)
        try:
            for frame in stub.StreamAllEvents(req, metadata=md):
                for d in frame.deltas:
                    # Every delta surfaces — an unrecognized kind arrives as
                    # ``"unknown"`` (the global-tail contract; TS/CLI parity).
                    yield types.GlobalDelta.from_proto(d)
                cursor = frame.next_seq
                if not follow and frame.journal_boundary:
                    return
        except grpc.RpcError as e:
            if follow and e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue  # CatchupRequired: resume from the last cursor
            raise from_rpc_error(e) from e
        if not follow:
            return


def stream_model_tokens(
    stub, md, instance_id: bytes, mote_id: bytes, since: int = 0
) -> Iterator[types.TokenChunk]:
    """Yield one model mote's ADVISORY tokens over the native gRPC stream (PR-4.2 /
    T-STREAM1): the NEW bytes per decode step until the terminal ``done`` chunk.
    The committed ``result_ref`` stays the authority — a consumer reconciles to it.
    An old gateway without this RPC raises (mapped via :func:`from_rpc_error`)."""
    req = _g.StreamModelTokensRequest(instance_id=instance_id, mote_id=mote_id, since_seq=since)
    try:
        for chunk in stub.StreamModelTokens(req, metadata=md):
            view = types.TokenChunk.from_proto(chunk)
            yield view
            if view.done:
                return
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


# --- async gRPC --------------------------------------------------------------


async def astream_deltas(
    stub, md, instance_id: bytes, since: int, follow: bool
) -> AsyncIterator[types.Delta]:
    cursor = since
    while True:
        req = _g.StreamEventsRequest(instance_id=instance_id, since_seq=cursor)
        try:
            call = stub.StreamEvents(req, metadata=md)
            async for frame in call:
                for d in frame.deltas:
                    view = types.Delta.from_proto(d)
                    if view is not None:
                        yield view
                cursor = frame.next_seq
                if not follow and frame.journal_boundary:
                    return
        except grpc.RpcError as e:
            if follow and e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue
            raise from_rpc_error(e) from e
        if not follow:
            return


async def astream_all_deltas(
    stub, md, since: int, follow: bool
) -> AsyncIterator[types.GlobalDelta]:
    cursor = since
    while True:
        req = _g.StreamAllEventsRequest(since_seq=cursor)
        try:
            call = stub.StreamAllEvents(req, metadata=md)
            async for frame in call:
                for d in frame.deltas:
                    # Every delta surfaces — an unrecognized kind arrives as
                    # ``"unknown"`` (the global-tail contract; TS/CLI parity).
                    yield types.GlobalDelta.from_proto(d)
                cursor = frame.next_seq
                if not follow and frame.journal_boundary:
                    return
        except grpc.RpcError as e:
            if follow and e.code() == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue
            raise from_rpc_error(e) from e
        if not follow:
            return


async def astream_model_tokens(
    stub, md, instance_id: bytes, mote_id: bytes, since: int = 0
) -> AsyncIterator[types.TokenChunk]:
    """Async twin of :func:`stream_model_tokens` (PR-4.2)."""
    req = _g.StreamModelTokensRequest(instance_id=instance_id, mote_id=mote_id, since_seq=since)
    try:
        call = stub.StreamModelTokens(req, metadata=md)
        async for chunk in call:
            view = types.TokenChunk.from_proto(chunk)
            yield view
            if view.done:
                return
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


# --- optional WebSocket bridge (R5) ------------------------------------------


def _ws_base(grpc_endpoint: str, ws_endpoint: Optional[str]) -> str:
    """The WS bridge base URL: an explicit ws endpoint, or the gRPC endpoint's
    scheme/host mapped to the conventional WS port (50152)."""
    if ws_endpoint:
        return ws_endpoint.rstrip("/")
    rest = grpc_endpoint
    scheme = "wss"
    if rest.startswith("http://"):
        scheme, rest = "ws", rest[len("http://") :]
    elif rest.startswith("https://"):
        scheme, rest = "wss", rest[len("https://") :]
    host = rest.split("/")[0].rsplit(":", 1)[0]
    return f"{scheme}://{host}:50152"


def _ws_url(grpc_endpoint: str, ws_endpoint: Optional[str], instance_hex: str, since: int) -> str:
    """Derive the per-run ``/v1/events`` WS URL."""
    return f"{_ws_base(grpc_endpoint, ws_endpoint)}/v1/events?instance={instance_hex}&since={since}"


def _ws_all_url(grpc_endpoint: str, ws_endpoint: Optional[str], since: int) -> str:
    """Derive the GLOBAL ``/v1/events/all`` WS URL (Batch C — no instance param)."""
    return f"{_ws_base(grpc_endpoint, ws_endpoint)}/v1/events/all?since={since}"


def _ws_token_url(
    grpc_endpoint: str, ws_endpoint: Optional[str], instance_hex: str, mote_hex: str, since: int
) -> str:
    """Derive the per-mote ``/v1/tokens`` WS URL (PR-4.2)."""
    base = _ws_base(grpc_endpoint, ws_endpoint)
    return f"{base}/v1/tokens?instance={instance_hex}&mote={mote_hex}&since={since}"


def _ws_delta(obj: dict) -> Optional[types.Delta]:
    """Map one R5 WS JSON delta (``type`` discriminant, hex ids) to a :class:`Delta`."""
    kind = obj.get("type")
    seq = int(obj.get("seq", 0))
    if kind == "committed":
        return types.Delta(
            seq=seq, kind="committed", mote_id=obj.get("mote_id"), result_ref=obj.get("result_ref")
        )
    if kind == "failed":
        return types.Delta(
            seq=seq, kind="failed", mote_id=obj.get("mote_id"), reason_class=obj.get("reason_class")
        )
    if kind == "repudiated":
        return types.Delta(
            seq=seq,
            kind="repudiated",
            target_mote_id=obj.get("target_mote_id"),
            target_committed_seq=obj.get("target_committed_seq"),
        )
    if kind == "effect_staged":
        return types.Delta(seq=seq, kind="effect_staged", mote_id=obj.get("mote_id"))
    return None


def ws_stream_deltas(
    grpc_endpoint: str,
    instance_hex: str,
    *,
    since: int = 0,
    token: Optional[str] = None,
    ws_endpoint: Optional[str] = None,
) -> Iterator[types.Delta]:
    """Consume the live tail over the R5 WebSocket bridge (requires ``kortecx[ws]``)."""
    import json

    try:
        from websockets.sync.client import connect
    except Exception as e:  # pragma: no cover
        raise ImportError(
            "the WebSocket events client needs the 'ws' extra: pip install 'kortecx[ws]'"
        ) from e

    url = _ws_url(grpc_endpoint, ws_endpoint, instance_hex, since)
    headers = {"Authorization": f"Bearer {token}"} if token else None
    with connect(url, additional_headers=headers) as ws:
        for message in ws:
            frame = json.loads(message)
            for d in frame.get("deltas", []):
                view = _ws_delta(d)
                if view is not None:
                    yield view


def ws_stream_model_tokens(
    grpc_endpoint: str,
    instance_hex: str,
    mote_hex: str,
    *,
    since: int = 0,
    token: Optional[str] = None,
    ws_endpoint: Optional[str] = None,
) -> Iterator[types.TokenChunk]:
    """Consume one model mote's ADVISORY token stream over the WS bridge (PR-4.2;
    requires ``kortecx[ws]``). Each WS message is exactly ONE chunk (not a frame).
    Stops after the terminal ``done`` chunk."""
    import json

    try:
        from websockets.sync.client import connect
    except Exception as e:  # pragma: no cover
        raise ImportError(
            "the WebSocket token client needs the 'ws' extra: pip install 'kortecx[ws]'"
        ) from e

    url = _ws_token_url(grpc_endpoint, ws_endpoint, instance_hex, mote_hex, since)
    headers = {"Authorization": f"Bearer {token}"} if token else None
    with connect(url, additional_headers=headers) as ws:
        for message in ws:
            view = types.TokenChunk.from_ws(json.loads(message))
            yield view
            if view.done:
                return


def _ws_global_delta(obj: dict) -> types.GlobalDelta:
    """Map one global WS JSON delta (``type`` discriminant, hex ids, per-delta
    ``instance_id`` attribution) to a :class:`GlobalDelta`. An unknown/future
    ``type`` SURFACES as ``"unknown"`` (never dropped — the global-tail
    contract, matching the TS parser + the CLI renderer; the per-run parser's
    silent skip is deliberately NOT mirrored here)."""
    kind = obj.get("type")
    seq = int(obj.get("seq", 0))
    inst = obj.get("instance_id") or ""  # "" before any registration
    if kind == "run_registered":
        return types.GlobalDelta(
            seq=seq,
            kind="run_registered",
            instance_id=inst,
            recipe_fingerprint=obj.get("recipe_fingerprint"),
            registered_unix_ms=obj.get("registered_unix_ms"),
        )
    if kind == "committed":
        return types.GlobalDelta(
            seq=seq,
            kind="committed",
            instance_id=inst,
            mote_id=obj.get("mote_id"),
            result_ref=obj.get("result_ref"),
            nd_class=obj.get("nd_class"),
        )
    if kind == "failed":
        return types.GlobalDelta(
            seq=seq,
            kind="failed",
            instance_id=inst,
            mote_id=obj.get("mote_id"),
            reason_class=obj.get("reason_class"),
        )
    if kind == "repudiated":
        return types.GlobalDelta(
            seq=seq,
            kind="repudiated",
            instance_id=inst,
            target_mote_id=obj.get("target_mote_id"),
            target_committed_seq=obj.get("target_committed_seq"),
        )
    if kind == "effect_staged":
        return types.GlobalDelta(
            seq=seq, kind="effect_staged", instance_id=inst, mote_id=obj.get("mote_id")
        )
    return types.GlobalDelta(seq=seq, kind="unknown", instance_id=inst)


def ws_stream_all_deltas(
    grpc_endpoint: str,
    *,
    since: int = 0,
    token: Optional[str] = None,
    ws_endpoint: Optional[str] = None,
) -> Iterator[types.GlobalDelta]:
    """Consume the GLOBAL live tail over the WS bridge (requires ``kortecx[ws]``)."""
    import json

    try:
        from websockets.sync.client import connect
    except Exception as e:  # pragma: no cover
        raise ImportError(
            "the WebSocket events client needs the 'ws' extra: pip install 'kortecx[ws]'"
        ) from e

    url = _ws_all_url(grpc_endpoint, ws_endpoint, since)
    headers = {"Authorization": f"Bearer {token}"} if token else None
    with connect(url, additional_headers=headers) as ws:
        for message in ws:
            frame = json.loads(message)
            for d in frame.get("deltas", []):
                yield _ws_global_delta(d)
