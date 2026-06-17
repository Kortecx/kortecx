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


# --- PR-6a declarative tools registry (DiscoverTools / RegisterTool) ------------
#
# DISTINCT from the advisory ToolManifest above: this is the durable registry
# INVENTORY (what is registered, by whom, with what authority). Registration
# grants NO authority — a tool fires only under a server-issued warrant (SN-8);
# `tool_id` is server-derived (the client never names/forges it). DIALING a
# registered external MCP server is a Cloud / PR-6b capability.


@dataclass(frozen=True)
class RegisteredTool:
    """One durable-registry row (``DiscoverTools``). ``net_scope`` is a display
    summary; authority never rides this wire (SN-8)."""

    tool_id: str  # 16-byte server-derived id, hex
    tool_name: str
    tool_version: str
    kind: str  # "Builtin" | "Mcp"
    description: str
    idempotency_class: str
    provenance: str  # "HumanAuthored" | "SelfGenerated"
    registration_status: str  # "Approved" | "PendingHumanReview"
    server_host: str  # the vetted egress endpoint (empty = no egress)
    net_scope: str  # "none" | "egress:host[,host]"
    is_builtin: bool

    @classmethod
    def from_proto(cls, t: "_g.RegisteredTool") -> "RegisteredTool":
        return cls(
            tool_id=hexids.encode(t.tool_id),
            tool_name=t.tool_name,
            tool_version=t.tool_version,
            kind=t.kind,
            description=t.description,
            idempotency_class=t.idempotency_class,
            provenance=t.provenance,
            registration_status=t.registration_status,
            server_host=t.server_host,
            net_scope=t.net_scope_summary,
            is_builtin=t.is_builtin,
        )


@dataclass(frozen=True)
class RegisteredToolsPage:
    """One ``DiscoverTools`` page (deterministic ``(name, version)`` order)."""

    tools: List[RegisteredTool]
    has_more: bool


@dataclass(frozen=True)
class McpServer:
    """One registered external MCP server (PR-6b-1 ``ListMcpServers``). The
    credential VALUE is never on the wire — only whether a ref NAME is attached
    (D81)."""

    connection_id: str  # 16-byte server-derived id, hex
    server_name: str
    transport: str  # "stdio" | "http"
    endpoint: str  # command (stdio) | URL (http)
    health: str  # "connected" | "unreachable" | "unknown"
    tool_count: int
    credential_ref_present: bool

    @classmethod
    def from_proto(cls, s: "_g.McpServer") -> "McpServer":
        return cls(
            connection_id=hexids.encode(s.connection_id),
            server_name=s.server_name,
            transport=s.transport,
            endpoint=s.endpoint,
            health=s.health,
            tool_count=s.tool_count,
            credential_ref_present=s.credential_ref_present,
        )


@dataclass(frozen=True)
class McpServersPage:
    """One ``ListMcpServers`` page (deterministic ``(name)`` order)."""

    servers: List[McpServer]
    has_more: bool


@dataclass(frozen=True)
class RegisterServerResult:
    """The outcome of ``register_mcp_server`` — the server-derived connection id,
    the count of tools discovered + registered, and the folded health."""

    connection_id: str  # 16-byte server-derived id, hex
    discovered: int
    health: str  # "connected" | "unreachable" | "unknown"


@dataclass(frozen=True)
class ToolParam:
    """A declared, typed tool input parameter (the MCP inputSchema analogue —
    CLOSED set, no float, SN-8). ``ty`` in ``str|bytes|int|bool|enum``."""

    name: str
    ty: str = "str"
    max_len: int = 0  # str/bytes byte cap (0 = server default)
    required: bool = True
    allowed: Sequence[str] = ()  # enum: permitted exact values

    def to_proto(self) -> "_g.ToolParamSpec":
        return _g.ToolParamSpec(
            name=self.name,
            ty=self.ty,
            max_len=self.max_len,
            required=self.required,
            allowed=list(self.allowed),
        )
