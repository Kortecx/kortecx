"""Hex helpers for server-derived identifiers.

The runtime computes every identifier (``MoteId``, ``instance_id``,
``content_ref``, ``terminal_mote_id``) — the SDK **never** derives one (SN-8).
These helpers only *encode* server bytes to lowercase hex for display and
*decode* a user-supplied hex string back to bytes (with strict length
validation). There is deliberately no "compute an id" function in this module
or anywhere else in the SDK.
"""

from __future__ import annotations

from .errors import KxUsage

#: Length in bytes of a run instance id.
INSTANCE_LEN = 16
#: Length in bytes of a content ref / Mote id / signature id / digest.
REF_LEN = 32


def encode(data: bytes) -> str:
    """Render server bytes as lowercase hex (the SDK's display form for ids)."""
    return data.hex()


def encode_opt(data: "bytes | None") -> "str | None":
    """:func:`encode`, but ``None``-preserving (for optional refs)."""
    return None if data is None else data.hex()


def decode(s: str) -> bytes:
    """Decode a hex string to bytes, raising :class:`KxUsage` on bad hex."""
    s = s.strip()
    try:
        return bytes.fromhex(s)
    except ValueError as e:
        raise KxUsage(f"invalid hex: {e}") from e


def decode_fixed(s: str, n: int) -> bytes:
    """Decode hex and require exactly ``n`` bytes (a length footgun guard)."""
    b = decode(s)
    if len(b) != n:
        raise KxUsage(f"expected {n} bytes ({n * 2} hex chars), got {len(b)} bytes")
    return b


def instance_id(s: str) -> bytes:
    """Decode a 16-byte run ``instance_id`` from hex."""
    return decode_fixed(s, INSTANCE_LEN)


def ref32(s: str) -> bytes:
    """Decode a 32-byte ref (content_ref / mote_id / signature_id) from hex."""
    return decode_fixed(s, REF_LEN)


def as_bytes(value: "str | bytes", n: int) -> bytes:
    """Accept either a hex string or raw ``n``-byte bytes; validate the length.

    This lets every SDK method take an id as the hex the rest of the SDK prints,
    *or* as the raw bytes a previous response carried — both server-derived.
    """
    if isinstance(value, str):
        return decode_fixed(value, n)
    if isinstance(value, (bytes, bytearray)):
        if len(value) != n:
            raise KxUsage(f"expected {n} bytes, got {len(value)}")
        return bytes(value)
    raise KxUsage(f"expected a hex str or {n} bytes, got {type(value).__name__}")
