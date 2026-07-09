"""``kortecx.appbundle/v1`` — the portable App archive codec.

An App bundle packages an App for portability: the canonical ``AppEnvelope`` bytes
plus the base64 closure of every content-store blob it references. The wire form is
a single canonical-JSON, all-strings document (sorted keys, compact) so this SDK,
the Rust ``kx-appbundle`` crate, and the TS SDK emit byte-identical bundles — the
cross-language contract is ``tests/golden/apps/bundle_corpus.json``.

This module owns the container format only. It validates structure — the schema
tag, 64-char lowercase-hex refs, and well-formed base64 — never cryptographic
identity: the runtime re-derives every blob ref and re-validates the envelope
server-side, so a bundle is a transport hint, never a trust boundary.
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass, field
from typing import Dict, Optional

#: The bundle schema/version tag — readers fail closed on a mismatch.
BUNDLE_SCHEMA = "kortecx.appbundle/v1"

#: Advisory import ceilings (H7) — bound the whole closure a bundle can carry
#: (distinct from the server's per-blob 32 MiB PutContent cap).
MAX_BUNDLE_REFS = 4096
MAX_BUNDLE_CLOSURE_BYTES = 512 * 1024 * 1024  # 512 MiB

_HEX = frozenset("0123456789abcdef")


def _check_hex(field_name: str, s: str) -> None:
    if len(s) != 64 or any(c not in _HEX for c in s):
        raise ValueError(f"{field_name} must be 64-char lowercase hex, got {s!r}")


@dataclass(frozen=True)
class AppBundle:
    """A decoded App bundle: the canonical envelope bytes + the raw content closure,
    named + tamper-checkable by the App's ``app_digest`` (verified by the runtime,
    not here). ``source_digest`` is an optional lineage hint (never authenticity)."""

    app_digest: str  # 64-hex handle-free App identity
    envelope: bytes  # canonical AppEnvelope bytes, verbatim
    blobs: Dict[str, bytes] = field(default_factory=dict)  # 64-hex ref -> raw bytes
    source_digest: Optional[str] = None  # 64-hex lineage hint, if imported/cloned

    def to_json(self) -> str:
        """Serialize to the canonical ``kortecx.appbundle/v1`` wire string (sorted
        keys, compact, base64-STANDARD blobs) — byte-identical across Rust/Py/TS."""
        doc: Dict[str, object] = {
            "app_digest": self.app_digest,
            "envelope": self.envelope.decode("utf-8"),
            "schema": BUNDLE_SCHEMA,
        }
        if self.blobs:
            # b64encode (NOT encodebytes) — STANDARD alphabet, padded, single-line.
            doc["blobs"] = {
                ref: base64.b64encode(body).decode("ascii") for ref, body in self.blobs.items()
            }
        if self.source_digest is not None:
            doc["source_digest"] = self.source_digest
        return json.dumps(doc, sort_keys=True, separators=(",", ":"), ensure_ascii=False)

    @classmethod
    def from_json(cls, wire: str) -> "AppBundle":
        """Parse + structurally validate a ``kortecx.appbundle/v1`` wire string. Does
        NOT verify a blob hashes to its ref or that the envelope is valid — the
        runtime re-derives + re-validates those server-side.

        Raises ``ValueError`` on a schema mismatch, a bad hex ref, or bad base64."""
        doc = json.loads(wire)
        schema = doc.get("schema")
        if schema != BUNDLE_SCHEMA:
            raise ValueError(
                f"unsupported app bundle schema {schema!r} (expected {BUNDLE_SCHEMA!r})"
            )
        app_digest = doc["app_digest"]
        _check_hex("app_digest", app_digest)
        source_digest = doc.get("source_digest")
        if source_digest is not None:
            _check_hex("source_digest", source_digest)
        blobs: Dict[str, bytes] = {}
        for ref, b64 in (doc.get("blobs") or {}).items():
            _check_hex("blob ref", ref)
            blobs[ref] = base64.b64decode(b64, validate=True)
        return cls(
            app_digest=app_digest,
            envelope=doc["envelope"].encode("utf-8"),
            blobs=blobs,
            source_digest=source_digest,
        )

    def total_blob_bytes(self) -> int:
        """Total raw byte size of the content closure (for an import ceiling)."""
        return sum(len(b) for b in self.blobs.values())

    def blob_count(self) -> int:
        """Number of blobs in the content closure."""
        return len(self.blobs)
