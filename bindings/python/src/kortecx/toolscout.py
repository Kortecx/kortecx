"""W1.A5 toolscout views — ADVISORY tool discovery + TaskBundle preview, as
surfaced by ``ListToolManifests`` / ``ScoreTaskBundle``.

Kept in its own module (the runs.py / module-per-concern precedent). SN-8: every
score/verdict here is DISPLAY-ONLY — never a committed fact, never an
authorization. The sole grant gate stays the exact ``(name, version)`` equality
check in lowering + the broker; a score can surface a tool, never grant one. The
verdict is a server-side DRY-RUN of the real lowering gate against the
SERVER-built react warrant (no client warrant input) — the lowered WorkflowDef
is discarded. Fingerprints are server-derived; the SDK only hex-encodes the bytes.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Sequence

from . import hexids
from .v1 import gateway_pb2 as _g

# --- verdict display names (mirror the LowerVerdict proto enum) ----------------

_VERDICT_NAMES: "dict[int, str]" = {
    _g.LowerVerdict.LOWER_VERDICT_UNAVAILABLE: "unavailable",
    _g.LowerVerdict.LOWER_VERDICT_WOULD_LOWER: "would-lower",
    _g.LowerVerdict.LOWER_VERDICT_REFUSED: "refused",
}


def lower_verdict_name(verdict: int) -> str:
    """Map a ``LowerVerdict`` discriminant to a stable lowercase name.
    ``"unknown"`` absorbs UNSPECIFIED(0) + any value this SDK predates (the
    ``state_name`` precedent, mirrored by the TS SDK — never a crash)."""
    return _VERDICT_NAMES.get(verdict, "unknown")


# --- manifest / score views -----------------------------------------------------


@dataclass(frozen=True)
class KeywordSet:
    """Normalized intent keywords under one BCP-47-ish language tag (advisory)."""

    lang: str  # e.g. "en", "hi", "ja"
    words: List[str]  # pre-normalized (trim / lowercase / collapse whitespace)

    @classmethod
    def from_proto(cls, k: "_g.KeywordSet") -> "KeywordSet":
        return cls(lang=k.lang, words=list(k.words))

    def to_proto(self) -> "_g.KeywordSet":
        return _g.KeywordSet(lang=self.lang, words=list(self.words))


@dataclass(frozen=True)
class ToolManifest:
    """One registered tool's ADVISORY manifest — ranking/display material ONLY
    (the broker never reads manifests). Identity is the exact
    ``(tool_id, tool_version)`` pair; ``fingerprint_hash`` is the display/join key."""

    tool_id: str
    tool_version: str
    description: str  # free-form human text; NEVER parsed for enforcement
    keywords: List[KeywordSet]
    fingerprint_hash: str  # hex (32B blake3 ToolFingerprint content hash)
    kind: str  # "Builtin" | "Mcp" (display)

    @classmethod
    def from_proto(cls, m: "_g.ToolManifest") -> "ToolManifest":
        return cls(
            tool_id=m.tool_id,
            tool_version=m.tool_version,
            description=m.description,
            keywords=[KeywordSet.from_proto(k) for k in m.keywords],
            fingerprint_hash=hexids.encode(m.fingerprint_hash),
            kind=m.kind,
        )


@dataclass(frozen=True)
class ManifestScore:
    """One manifest's advisory rank against a bundle intent, in integer basis
    points (SN-8: floats never cross the wire; a score never authorizes)."""

    tool_id: str
    tool_version: str
    score_bp: int  # 0..=10000 basis points
    fingerprint_hash: str  # hex — joins back to ListToolManifests

    @classmethod
    def from_proto(cls, s: "_g.ManifestScore") -> "ManifestScore":
        return cls(
            tool_id=s.tool_id,
            tool_version=s.tool_version,
            score_bp=s.score_bp,
            fingerprint_hash=hexids.encode(s.fingerprint_hash),
        )


@dataclass(frozen=True)
class BundleScore:
    """A ``ScoreTaskBundle`` outcome: every registered manifest ranked best-first
    + the lowering-gate DRY-RUN verdict (``"unavailable"`` | ``"would-lower"`` |
    ``"refused"``; ``"unknown"`` absorbs any future value). DISPLAY-ONLY (SN-8)
    — the broker re-gates any real dispatch."""

    bundle_fingerprint: str  # hex (32B blake3 TaskBundle content fingerprint)
    ranked: List[ManifestScore]
    verdict: str  # "unavailable" | "would-lower" | "refused" | "unknown"
    verdict_detail: str  # display-only availability/refusal prose

    @classmethod
    def from_proto(cls, r: "_g.ScoreTaskBundleResponse") -> "BundleScore":
        return cls(
            bundle_fingerprint=hexids.encode(r.bundle_fingerprint),
            ranked=[ManifestScore.from_proto(s) for s in r.ranked],
            verdict=lower_verdict_name(r.verdict),
            verdict_detail=r.verdict_detail,
        )


# --- bundle-spec inputs ----------------------------------------------------------


@dataclass
class BundleTool:
    """One sequenced tool in a client-authored TaskBundle spec. Identity is the
    exact ``(tool_id, tool_version)`` pair; description/keywords are advisory
    ToolMeta that rides along."""

    tool_id: str
    tool_version: str
    description: str = ""
    keywords: Sequence[KeywordSet] = ()

    def to_proto(self) -> "_g.BundleToolSpec":
        return _g.BundleToolSpec(
            tool_id=self.tool_id,
            tool_version=self.tool_version,
            description=self.description,
            keywords=[k.to_proto() for k in self.keywords],
        )


@dataclass
class BundleSpec:
    """A client-authored TaskBundle to score: the task ``intent``, the ordered
    ``tools`` sequence, optional advisory ``language_tags``, and the advisory
    ranking cut ``tolerance_threshold_bp`` (an integer, 0..=10000 — never a
    float). The server validates fail-closed (size/count caps, duplicate names
    refused)."""

    intent: str
    tools: Sequence[BundleTool]
    language_tags: Sequence[str] = ()
    tolerance_threshold_bp: int = 0

    def to_proto(self) -> "_g.ScoreTaskBundleRequest":
        return _g.ScoreTaskBundleRequest(
            intent=self.intent,
            language_tags=list(self.language_tags),
            tool_sequence=[t.to_proto() for t in self.tools],
            tolerance_threshold_bp=self.tolerance_threshold_bp,
        )
