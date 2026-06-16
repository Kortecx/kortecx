"""Operator alerts inbox view — one ``AlertSummary`` enumerated by ``ListAlerts``
(W1a-2).

A read-only projection of the journal's TERMINAL ``Failed`` facts (dead-letters +
worker-reported terminal failures) into a rebuildable-to-empty ``alerts.db``
read-cache — display/triage-read ONLY, never truth, never identity, never a
digest input. Serve-path admission refusals write nothing to the journal, so
they are not in this inbox. Kept in its own module (the telemetry.py /
module-per-concern precedent). SN-8: ``alert_id``/``mote_id`` are server-derived;
the SDK only hex-encodes the bytes.

The triage LIFECYCLE (acknowledge/resolve), the alert-rule engine, and
notifications are a Cloud capability (D156/D129) — there is no mutate method here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class AlertSummary:
    """One terminal-failure alert: hex ids + the failure class + a display
    severity + the ``Failed`` fact's journal seq (the deep-link cursor +
    pagination). ``instance_id`` is registration-watermark attribution — all-zero
    hex when unattributed."""

    alert_id: str  # hex (server-derived, re-fold-stable)
    mote_id: str  # hex (the failed Mote; deep-link target)
    instance_id: str  # hex (all-zero = unattributed)
    reason_class: str  # terminal FailureReason wire token (e.g. "dead_lettered")
    reason_code: int  # numeric FailureReason discriminant (0-8)
    severity: str  # closed display vocabulary: "error" | "refused"
    seq: int  # the Failed fact's journal seq
    created_unix_ms: int  # audit-only first-folded wall clock (off every hash)

    @classmethod
    def from_proto(cls, a: "_g.AlertSummary") -> "AlertSummary":
        return cls(
            alert_id=hexids.encode(a.alert_id),
            mote_id=hexids.encode(a.mote_id),
            instance_id=hexids.encode(a.instance_id),
            reason_class=a.reason_class,
            reason_code=a.reason_code,
            severity=a.severity,
            seq=a.seq,
            created_unix_ms=a.created_unix_ms,
        )


@dataclass(frozen=True)
class AlertsPage:
    """One newest-first page of :class:`AlertSummary` plus the ``has_more`` flag."""

    alerts: List[AlertSummary]
    has_more: bool
