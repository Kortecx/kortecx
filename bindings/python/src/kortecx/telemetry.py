"""Mote execution telemetry view — one ``MoteTelemetryRow`` enumerated by
``ListMoteTelemetry`` (PR-3 Monitoring, Batch C).

The execution exhaust the HOST records as motes actually run: wall-clock, model
usage, the fired tool. Lives in a rebuildable-to-empty ``telemetry.db`` sidecar
— audit/display ONLY, never truth, never identity, never a digest input. Kept
in its own module (the runs.py / module-per-concern precedent). SN-8: ids are
server-derived; the SDK only hex-encodes the bytes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class MoteTelemetryRow:
    """One executed Mote's telemetry: hex join keys + host-measured wall time +
    model/tool usage + the Committed fact's journal seq (the pagination cursor).
    ``instance_id`` is registration-watermark attribution — all-zero hex when
    unattributed."""

    mote_id: str  # hex
    instance_id: str  # hex (all-zero = unattributed)
    wall_clock_ms: int
    input_tokens: Optional[int]  # NEVER set in OSS: the backend seam reports no input count
    output_tokens: Optional[int]  # model motes on an inference build only; None otherwise
    model_id: str  # the model that ACTUALLY ran ("" for non-model motes / FFI-free)
    tool_id: str  # the pinned tool of a tool-bearing mote (else "")
    started_unix_ms: int  # audit-only start wall clock (off every hash)
    seq: int

    @classmethod
    def from_proto(cls, r: "_g.MoteTelemetryRow") -> "MoteTelemetryRow":
        return cls(
            mote_id=hexids.encode(r.mote_id),
            instance_id=hexids.encode(r.instance_id),
            wall_clock_ms=r.wall_clock_ms,
            input_tokens=r.input_tokens if r.HasField("input_tokens") else None,
            output_tokens=r.output_tokens if r.HasField("output_tokens") else None,
            model_id=r.model_id,
            tool_id=r.tool_id,
            started_unix_ms=r.started_unix_ms,
            seq=r.seq,
        )


@dataclass(frozen=True)
class TelemetryPage:
    """One newest-first page of :class:`MoteTelemetryRow` plus the ``has_more`` flag."""

    rows: List[MoteTelemetryRow]
    has_more: bool
