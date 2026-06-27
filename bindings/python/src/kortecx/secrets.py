"""Operator secret-store admin (D170 / MM-3) — the named secret VALUES the runtime
holds for connector credentials + trigger auth, surfaced by ``PutSecret`` /
``ListSecretNames`` / ``DeleteSecret``.

Kept in its own module (the feedback.py / module-per-concern precedent, GR3). The
secret VALUE never crosses back over the wire (D81): ``ListSecretNames`` returns
only NAMES + audit timestamps. A connector ``credential_ref`` / a trigger
``auth_secret_ref`` NAMES one of these rows; the value resolves server-side at
dial / fire time, never in the client.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class SecretName:
    """One stored secret's metadata (``ListSecretNames``) — NAME + audit
    timestamps only. The secret VALUE is never on this wire (D81); the timestamps
    are display/audit wall clock (never identity, never a hash input)."""

    name: str
    created_unix_ms: int
    updated_unix_ms: int

    @classmethod
    def from_proto(cls, s: "_g.SecretName") -> "SecretName":
        return cls(
            name=s.name,
            created_unix_ms=s.created_unix_ms,
            updated_unix_ms=s.updated_unix_ms,
        )


@dataclass(frozen=True)
class SecretNamesPage:
    """One ``ListSecretNames`` page (deterministic ``(name)`` order)."""

    names: List[SecretName]
    has_more: bool
