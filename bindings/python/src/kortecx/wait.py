"""``wait`` orchestration — turn an async run handle into a single result.

This composes existing RPCs client-side, exactly like the ``kx`` CLI
(``wait.rs``): poll ``GetProjection`` until the target Mote is terminal, then
``GetContent`` its committed result. Two strategies share one :class:`WaitOutcome`:

* **poll** (default, CLI-parity) — re-read the projection every 250 ms.
* **events** (opt-in, lower latency) — subscribe to ``StreamEvents`` and react to
  the terminal Mote's delta as it lands (sub-second), resuming on a CatchupRequired
  drop. This is strictly more efficient than the poll and is forward-compatible
  with the live tail.

Both sync (``poll_*`` / ``events_*``) and async (``apoll_*`` / ``aevents_*``)
variants exist; they operate on the same generated ``KxGatewayStub`` (the channel
kind — sync or aio — decides whether a call returns a value or an awaitable).
"""

from __future__ import annotations

import asyncio
import time
from dataclasses import dataclass
from enum import Enum
from typing import List, Optional

import grpc

from . import types
from .errors import from_rpc_error
from .v1 import gateway_pb2 as _g

#: Polling cadence — matches the CLI's bounded backoff (never a busy spin).
POLL_INTERVAL = 0.25


class WaitState(str, Enum):
    """The terminal disposition of a waited-on run."""

    COMMITTED = "COMMITTED"
    FAILED = "FAILED"
    RUNNING = "RUNNING"  # timed out, still in progress, resumable


@dataclass
class WaitOutcome:
    """Server-derived ids + the terminal disposition (+ result on commit)."""

    instance_id: bytes
    terminal_mote_id: bytes
    state: WaitState
    result_ref: Optional[bytes] = None
    payload: Optional[bytes] = None


def _terminal(instance_id: bytes, mote_id: bytes, state: WaitState) -> WaitOutcome:
    return WaitOutcome(instance_id=instance_id, terminal_mote_id=mote_id, state=state)


def _snapshot_result_ref(m: "_g.MoteSnapshot") -> Optional[bytes]:
    return m.result_ref if m.HasField("result_ref") else None


# --- sync (poll) --------------------------------------------------------------


def _get_projection(stub, md, instance_id: bytes) -> "_g.ProjectionView":
    try:
        return stub.GetProjection(_g.GetProjectionRequest(instance_id=instance_id), metadata=md)
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


def _get_content(stub, md, instance_id: bytes, content_ref: bytes) -> bytes:
    try:
        return stub.GetContent(
            _g.GetContentRequest(content_ref=content_ref, instance_id=instance_id),
            metadata=md,
        ).payload
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


def _committed_outcome(stub, md, instance_id, mote_id, result_ref) -> WaitOutcome:
    payload = _get_content(stub, md, instance_id, result_ref) if result_ref else None
    return WaitOutcome(
        instance_id=instance_id,
        terminal_mote_id=mote_id,
        state=WaitState.COMMITTED,
        result_ref=result_ref,
        payload=payload,
    )


def poll_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    """Poll until ``terminal_mote_id`` is terminal (the ``invoke`` path)."""
    deadline = time.monotonic() + timeout
    while True:
        view = _get_projection(stub, md, instance_id)
        m = next((x for x in view.motes if x.mote_id == terminal_mote_id), None)
        if m is not None:
            if types.is_committed(m.state):
                return _committed_outcome(
                    stub, md, instance_id, terminal_mote_id, _snapshot_result_ref(m)
                )
            if not types.is_pending(m.state):
                return _terminal(instance_id, terminal_mote_id, WaitState.FAILED)
        if time.monotonic() >= deadline:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        time.sleep(POLL_INTERVAL)


#: The branches a ReAct turn settles to (vs the live "pending"/"tool" states).
_REACT_ANSWER = "answer"
_REACT_DEAD = "dead_lettered"


def _list_react_turns(stub, md, instance_id: bytes):
    try:
        resp = stub.ListReactTurns(
            _g.ListReactTurnsRequest(instance_id=instance_id), metadata=md
        )
        return list(resp.turns)
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


def _projection_result_ref(stub, md, instance_id: bytes, mote_id: bytes) -> Optional[bytes]:
    view = _get_projection(stub, md, instance_id)
    m = next((x for x in view.motes if x.mote_id == mote_id), None)
    return _snapshot_result_ref(m) if m is not None else None


def poll_react_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    """Wait for a ReAct CHAIN to settle (the ``invoke`` react path).

    A react chain has no statically-known terminal Mote: the run-salted turn-0 id
    the gateway hands back never matches the committed turn id, and the settled
    Answer turn isn't known until the model emits it. So completion is observed via
    ``ListReactTurns`` — the chain is done when a turn settles to ``answer`` (the
    final answer; resolve its committed content) or ``dead_lettered`` (terminal
    failure). This is the durable, server-derived signal (the runtime's own
    "resume with get_projection / events" hint, made the default for react).
    """
    deadline = time.monotonic() + timeout
    while True:
        turns = _list_react_turns(stub, md, instance_id)
        answer = next((t for t in turns if t.branch == _REACT_ANSWER), None)
        if answer is not None:
            rr = _projection_result_ref(stub, md, instance_id, answer.turn_mote_id)
            return _committed_outcome(stub, md, instance_id, answer.turn_mote_id, rr)
        dead = next((t for t in turns if t.branch == _REACT_DEAD), None)
        if dead is not None:
            return _terminal(instance_id, dead.turn_mote_id, WaitState.FAILED)
        if time.monotonic() >= deadline:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        time.sleep(POLL_INTERVAL)


def poll_any(stub, md, instance_id, timeout) -> WaitOutcome:
    """Poll until ANY Mote commits (the ``submit`` path — no terminal id)."""
    deadline = time.monotonic() + timeout
    while True:
        view = _get_projection(stub, md, instance_id)
        committed = next((x for x in view.motes if types.is_committed(x.state)), None)
        if committed is not None:
            return _committed_outcome(
                stub, md, instance_id, committed.mote_id, _snapshot_result_ref(committed)
            )
        if view.motes and all(not types.is_pending(x.state) for x in view.motes):
            first = view.motes[0].mote_id
            return _terminal(instance_id, first, WaitState.FAILED)
        if time.monotonic() >= deadline:
            return _terminal(instance_id, b"", WaitState.RUNNING)
        time.sleep(POLL_INTERVAL)


# --- sync (events) ------------------------------------------------------------


def events_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    """Wait via the live event stream (lower latency than the poll).

    Subscribes from ``seq 0`` (so an already-committed terminal Mote is seen in the
    catch-up replay), watches for the terminal Mote's committed/failed delta, and
    resumes from the last cursor on a CatchupRequired drop.
    """
    deadline = time.monotonic() + timeout
    cursor = 0
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        req = _g.StreamEventsRequest(instance_id=instance_id, since_seq=cursor)
        try:
            for frame in stub.StreamEvents(req, metadata=md, timeout=remaining):
                for d in frame.deltas:
                    which = d.WhichOneof("kind")
                    if which == "committed" and d.committed.mote_id == terminal_mote_id:
                        rr = d.committed.result_ref or None
                        return _committed_outcome(stub, md, instance_id, terminal_mote_id, rr)
                    if which == "failed" and d.failed.mote_id == terminal_mote_id:
                        return _terminal(instance_id, terminal_mote_id, WaitState.FAILED)
                cursor = frame.next_seq
        except grpc.RpcError as e:
            code = e.code()
            if code == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue  # CatchupRequired: resume from the last cursor
            if code == grpc.StatusCode.DEADLINE_EXCEEDED:
                return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
            raise from_rpc_error(e) from e


# --- async (poll) -------------------------------------------------------------


async def _aget_projection(stub, md, instance_id: bytes) -> "_g.ProjectionView":
    try:
        return await stub.GetProjection(
            _g.GetProjectionRequest(instance_id=instance_id), metadata=md
        )
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


async def _aget_content(stub, md, instance_id, content_ref) -> bytes:
    try:
        resp = await stub.GetContent(
            _g.GetContentRequest(content_ref=content_ref, instance_id=instance_id),
            metadata=md,
        )
        return resp.payload
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


async def _acommitted_outcome(stub, md, instance_id, mote_id, result_ref) -> WaitOutcome:
    payload = await _aget_content(stub, md, instance_id, result_ref) if result_ref else None
    return WaitOutcome(
        instance_id=instance_id,
        terminal_mote_id=mote_id,
        state=WaitState.COMMITTED,
        result_ref=result_ref,
        payload=payload,
    )


async def apoll_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    while True:
        view = await _aget_projection(stub, md, instance_id)
        m = next((x for x in view.motes if x.mote_id == terminal_mote_id), None)
        if m is not None:
            if types.is_committed(m.state):
                return await _acommitted_outcome(
                    stub, md, instance_id, terminal_mote_id, _snapshot_result_ref(m)
                )
            if not types.is_pending(m.state):
                return _terminal(instance_id, terminal_mote_id, WaitState.FAILED)
        if loop.time() >= deadline:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        await asyncio.sleep(POLL_INTERVAL)


async def _alist_react_turns(stub, md, instance_id: bytes):
    try:
        resp = await stub.ListReactTurns(
            _g.ListReactTurnsRequest(instance_id=instance_id), metadata=md
        )
        return list(resp.turns)
    except grpc.RpcError as e:
        raise from_rpc_error(e) from e


async def _aprojection_result_ref(stub, md, instance_id: bytes, mote_id: bytes) -> Optional[bytes]:
    view = await _aget_projection(stub, md, instance_id)
    m = next((x for x in view.motes if x.mote_id == mote_id), None)
    return _snapshot_result_ref(m) if m is not None else None


async def apoll_react_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    """Async mirror of :func:`poll_react_result` (the react ``invoke`` path)."""
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    while True:
        turns = await _alist_react_turns(stub, md, instance_id)
        answer = next((t for t in turns if t.branch == _REACT_ANSWER), None)
        if answer is not None:
            rr = await _aprojection_result_ref(stub, md, instance_id, answer.turn_mote_id)
            return await _acommitted_outcome(stub, md, instance_id, answer.turn_mote_id, rr)
        dead = next((t for t in turns if t.branch == _REACT_DEAD), None)
        if dead is not None:
            return _terminal(instance_id, dead.turn_mote_id, WaitState.FAILED)
        if loop.time() >= deadline:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        await asyncio.sleep(POLL_INTERVAL)


async def apoll_any(stub, md, instance_id, timeout) -> WaitOutcome:
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    while True:
        view = await _aget_projection(stub, md, instance_id)
        committed = next((x for x in view.motes if types.is_committed(x.state)), None)
        if committed is not None:
            return await _acommitted_outcome(
                stub, md, instance_id, committed.mote_id, _snapshot_result_ref(committed)
            )
        if view.motes and all(not types.is_pending(x.state) for x in view.motes):
            return _terminal(instance_id, view.motes[0].mote_id, WaitState.FAILED)
        if loop.time() >= deadline:
            return _terminal(instance_id, b"", WaitState.RUNNING)
        await asyncio.sleep(POLL_INTERVAL)


# --- async (events) -----------------------------------------------------------


async def aevents_result(stub, md, instance_id, terminal_mote_id, timeout) -> WaitOutcome:
    loop = asyncio.get_event_loop()
    deadline = loop.time() + timeout
    cursor = 0
    while True:
        remaining = deadline - loop.time()
        if remaining <= 0:
            return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
        req = _g.StreamEventsRequest(instance_id=instance_id, since_seq=cursor)
        call = stub.StreamEvents(req, metadata=md, timeout=remaining)
        try:
            async for frame in call:
                for d in frame.deltas:
                    which = d.WhichOneof("kind")
                    if which == "committed" and d.committed.mote_id == terminal_mote_id:
                        rr = d.committed.result_ref or None
                        return await _acommitted_outcome(
                            stub, md, instance_id, terminal_mote_id, rr
                        )
                    if which == "failed" and d.failed.mote_id == terminal_mote_id:
                        return _terminal(instance_id, terminal_mote_id, WaitState.FAILED)
                cursor = frame.next_seq
        except grpc.RpcError as e:
            code = e.code()
            if code == grpc.StatusCode.RESOURCE_EXHAUSTED:
                continue
            if code == grpc.StatusCode.DEADLINE_EXCEEDED:
                return _terminal(instance_id, terminal_mote_id, WaitState.RUNNING)
            raise from_rpc_error(e) from e


def collect_frames(frames: List["_g.EventFrame"]) -> List[types.Frame]:
    """Map raw proto frames to views (used by the events helpers + tests)."""
    return [types.Frame.from_proto(f) for f in frames]
