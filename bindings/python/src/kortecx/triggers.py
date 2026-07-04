"""Trigger admin (D170 / D113) — durable webhook / cron / gRPC triggers that bind
an inbound event to a published recipe, surfaced by ``RegisterTrigger`` /
``ListTriggers`` / ``DeregisterTrigger`` / ``SubmitTrigger`` / ``TestTrigger``.

Kept in its own module (the feedback.py / module-per-concern precedent, GR3). The
``trigger_id`` + the bound ``instance_id`` are server-derived (the SDK only
hex-encodes the bytes); the kind / auth enums map to/from stable lowercase strings
(the ``rating_to_proto`` precedent — an unknown string is rejected fail-closed
with a ``ValueError``). The auth SECRET value is never on the wire — only whether
a ref NAME is attached (``auth_secret_present``, D81).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from . import hexids
from .v1 import gateway_pb2 as _g

# --- kind / auth enum <-> stable lowercase string -------------------------------

_KIND_TO_PROTO: "dict[str, _g.TriggerKind]" = {
    "webhook": _g.TriggerKind.WEBHOOK,
    "cron": _g.TriggerKind.CRON,
    "grpc": _g.TriggerKind.GRPC,
}
_KIND_NAMES: "dict[int, str]" = {v: k for k, v in _KIND_TO_PROTO.items()}

_AUTH_TO_PROTO: "dict[str, _g.TriggerAuth]" = {
    "none": _g.TriggerAuth.NONE,
    "hmac_sha256": _g.TriggerAuth.HMAC_SHA256,
    "bearer": _g.TriggerAuth.BEARER,
}
_AUTH_NAMES: "dict[int, str]" = {v: k for k, v in _AUTH_TO_PROTO.items()}


def trigger_kind_to_proto(kind: str) -> "_g.TriggerKind":
    """Map a friendly kind string (``"webhook"`` | ``"cron"`` | ``"grpc"``) to the
    proto ``TriggerKind`` enum. An unknown string is rejected fail-closed."""
    try:
        return _KIND_TO_PROTO[kind]
    except KeyError:
        raise ValueError(f"kind must be one of {sorted(_KIND_TO_PROTO)}, got {kind!r}") from None


def trigger_auth_to_proto(auth: str) -> "_g.TriggerAuth":
    """Map a friendly auth string (``"none"`` | ``"hmac_sha256"`` | ``"bearer"``)
    to the proto ``TriggerAuth`` enum. An unknown string is rejected fail-closed."""
    try:
        return _AUTH_TO_PROTO[auth]
    except KeyError:
        raise ValueError(f"auth must be one of {sorted(_AUTH_TO_PROTO)}, got {auth!r}") from None


def trigger_kind_name(kind: int) -> str:
    """Map a ``TriggerKind`` discriminant back to its stable lowercase name.
    ``"unknown"`` absorbs UNSPECIFIED(0) + any value this SDK predates (the
    ``lower_verdict_name`` precedent — never a crash)."""
    return _KIND_NAMES.get(kind, "unknown")


def trigger_auth_name(auth: int) -> str:
    """Map a ``TriggerAuth`` discriminant back to its stable lowercase name.
    ``"unknown"`` absorbs UNSPECIFIED(0) + any value this SDK predates (the
    ``lower_verdict_name`` precedent — never a crash)."""
    return _AUTH_NAMES.get(auth, "unknown")


# --- views ----------------------------------------------------------------------


@dataclass(frozen=True)
class TriggerView:
    """One registered trigger (``ListTriggers``). ``trigger_id`` is server-derived
    (hex); ``kind`` / ``auth`` are stable lowercase names; the auth SECRET value is
    never on the wire — only ``auth_secret_present`` (D81). ``last_fire_unix_ms``
    is audit-only wall clock (``0`` = never fired)."""

    trigger_id: str  # server-derived id, hex
    name: str
    kind: str  # "webhook" | "cron" | "grpc" | "unknown"
    recipe_handle: str  # "" for an App target
    app_handle: str  # T-APP-TRIGGER-TARGET: the App target ("" for a recipe target)
    auth: str  # "none" | "hmac_sha256" | "bearer" | "unknown"
    auth_secret_present: bool
    schedule_spec: str  # interval seconds OR a 5-field crontab expr (cron kind) / "" otherwise
    timezone: str  # IANA zone for a 5-field cron expr ("" ⇒ UTC)
    enabled: bool
    require_approval: bool  # per-trigger HITL posture (D114)
    last_fire_unix_ms: int

    @classmethod
    def from_proto(cls, t: "_g.TriggerView") -> "TriggerView":
        return cls(
            trigger_id=hexids.encode(t.trigger_id),
            name=t.name,
            kind=trigger_kind_name(t.kind),
            recipe_handle=t.recipe_handle,
            app_handle=t.app_handle,
            auth=trigger_auth_name(t.auth),
            auth_secret_present=t.auth_secret_present,
            schedule_spec=t.schedule_spec,
            timezone=t.timezone,
            enabled=t.enabled,
            require_approval=t.require_approval,
            last_fire_unix_ms=t.last_fire_unix_ms,
        )


@dataclass(frozen=True)
class TriggersPage:
    """One ``ListTriggers`` page (deterministic ``(name)`` order)."""

    triggers: List[TriggerView]
    has_more: bool
