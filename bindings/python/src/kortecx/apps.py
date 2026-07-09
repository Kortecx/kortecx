"""POC-4 App-catalog views — a durable, reusable App (a ``kortecx.app/v1``
envelope: a portable blueprint wrapped with by-reference references, a 4-axis
steering config, and per-step replay intent).

Kept in its own module so ``types.py`` stays a thin aggregator (the Rust core's
module-per-concern discipline, GR3). SN-8: ``app_ref`` is SERVER-DERIVED (blake3
over the canonical envelope) — the client names a handle, never an identity. The
catalog lives in an off-journal ``apps.db`` sidecar (rebuildable-to-empty), scoped
to the authoring party; a not-found / not-owned App is UNIFORM (no cross-party
existence oracle). The envelope carries NO authority — ``run_app`` re-compiles the
blueprint and the server re-resolves every warrant from the caller's own grants.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Dict, List, Mapping

from . import hexids
from .v1 import gateway_pb2 as _g

#: The envelope schema/version tag — readers fail closed on a mismatch.
APP_SCHEMA = "kortecx.app/v1"


def canonical_json(envelope: Mapping[str, Any]) -> bytes:
    """The canonical envelope bytes: keys sorted, compact, UTF-8 — byte-identical to
    the Rust ``kx-app`` serializer and the TS SDK (the cross-surface gate,
    ``tests/golden/apps/SPEC.md``)."""
    return json.dumps(envelope, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def pretty_json(envelope: Mapping[str, Any]) -> str:
    """The human export form: pretty (2-space) + sorted keys + a trailing newline."""
    return json.dumps(envelope, sort_keys=True, indent=2, ensure_ascii=False) + "\n"


def content_refs(envelope: Mapping[str, Any], *, include_datasets: bool = False) -> List[str]:
    """Every content-store ref an App envelope references — the transitive export
    closure (and the seed for the future GC reachability set). Returns sorted,
    deduplicated 64-hex refs. ``include_datasets`` gates the (potentially large) RAG
    dataset payload refs. Mirrors Rust ``AppEnvelope::content_refs`` byte-for-byte."""
    refs: set[str] = set()
    references = envelope.get("references", {}) or {}
    for item in references.get("context", []) or []:
        if item.get("content_ref"):
            refs.add(item["content_ref"])
    for rail in ("prompts", "rules", "memory"):
        for item in references.get(rail, []) or []:
            if item.get("content_ref"):
                refs.add(item["content_ref"])
    for item in references.get("skills", []) or []:
        if item.get("instructions_ref"):
            refs.add(item["instructions_ref"])
    steering_context = (envelope.get("steering_config", {}) or {}).get("context", {}) or {}
    for ref in steering_context.get("context_refs", []) or []:
        if ref:
            refs.add(ref)
    if include_datasets:
        for dataset in references.get("datasets", []) or []:
            for ref in dataset.get("cas_refs", []) or []:
                if ref:
                    refs.add(ref)
    return sorted(refs)


def default_handle(name: str) -> str:
    """Derive the default 3-segment catalog handle ``apps/local/<sanitized>`` from an
    App name (mirrors the ``kx app`` CLI). Lowercases, maps invalid chars to ``-``,
    trims, and falls back to ``app``."""
    san = "".join(
        c if (c.islower() or c.isdigit() or c in "._-") else ("-" if not c.isupper() else c.lower())
        for c in name
    ).strip(".-")[:128]
    return f"apps/local/{san or 'app'}"


@dataclass(frozen=True)
class AppSummary:
    """An App's catalog/display view (the envelope-derived summary + server id)."""

    handle: str  # the "namespace/collection/name" catalog key
    app_ref: str  # server-derived canonical-envelope hash, as hex (16 bytes ⇒ 32 hex)
    name: str
    version: str
    description: str
    tags: List[str]
    step_count: int
    locked: bool = False  # POC-5b: the App's project branch is locked (edits refused)

    @classmethod
    def from_proto(cls, s: "_g.AppSummary") -> "AppSummary":
        return cls(
            handle=s.handle,
            app_ref=hexids.encode(s.app_ref),
            name=s.name,
            version=s.version,
            description=s.description,
            tags=list(s.tags),
            step_count=s.step_count,
            locked=s.locked,
        )


@dataclass(frozen=True)
class AppCapability:
    """One capability line in an :class:`AppManifest` (a tool or a connection)."""

    id: str  # tool id, or a connection descriptor
    version: str  # tool version; "" for a connection
    requested: bool  # the App named this capability
    in_policy: bool  # within your policy (fireable+registered tool / registered connection)
    inherited: bool  # surfaced only because the tool reach is inherit_principal

    @classmethod
    def from_proto(cls, c: "_g.AppCapability") -> "AppCapability":
        return cls(
            id=c.id,
            version=c.version,
            requested=c.requested,
            in_policy=c.in_policy,
            inherited=c.inherited,
        )


@dataclass(frozen=True)
class AppManifest:
    """A stored App's READ-ONLY capability manifest ("what this App needs vs. what you
    have"): the requested tools/connections/model diffed against your live policy. It
    gates nothing — the runtime enforces the same intersection at run (SN-8)."""

    reach_inherit: bool  # the App inherits your whole tool ceiling (reach=inherit_principal)
    tools: List[AppCapability]
    connections: List[AppCapability]
    model_route: str  # the App's declared route ("" ⇒ served default)
    model_route_served: bool  # the route is offered here (False ⇒ a run would refuse)

    @classmethod
    def from_proto(cls, r: "_g.GetAppManifestResponse") -> "AppManifest":
        return cls(
            reach_inherit=r.reach_inherit,
            tools=[AppCapability.from_proto(c) for c in r.tools],
            connections=[AppCapability.from_proto(c) for c in r.connections],
            model_route=r.model_route,
            model_route_served=r.model_route_served,
        )


@dataclass(frozen=True)
class ScaffoldLaunch:
    """POC-5a: the result of launching a server-side App scaffold (correlate by
    ``branch_handle`` — poll :class:`ScaffoldStatus`)."""

    branch_handle: str
    resumed: bool

    @classmethod
    def from_proto(cls, r: "_g.ScaffoldAppResponse") -> "ScaffoldLaunch":
        return cls(branch_handle=r.branch_handle, resumed=r.resumed)


_SCAFFOLD_PHASE_NAMES = {1: "planning", 2: "writing", 3: "done", 4: "failed"}


@dataclass(frozen=True)
class ScaffoldStatus:
    """POC-5a: the live scaffold status (phase + the done/pending skeleton files)."""

    phase: str  # "planning" | "writing" | "done" | "failed" | "unspecified"
    files_done: List[str]
    files_pending: List[str]
    detail: str

    @classmethod
    def from_proto(cls, r: "_g.GetScaffoldStatusResponse") -> "ScaffoldStatus":
        return cls(
            phase=_SCAFFOLD_PHASE_NAMES.get(r.phase, "unspecified"),
            files_done=list(r.files_done),
            files_pending=list(r.files_pending),
            detail=r.detail,
        )


@dataclass(frozen=True)
class SaveAppResult:
    """The outcome of a ``SaveApp`` upsert (server-derived ref + dedup signal)."""

    app_ref: str  # server-derived canonical-envelope hash, as hex
    handle: str  # echoed canonical handle
    deduplicated: bool  # an identical canonical envelope was already bound here

    @classmethod
    def from_proto(cls, r: "_g.SaveAppResponse") -> "SaveAppResult":
        return cls(
            app_ref=hexids.encode(r.app_ref),
            handle=r.handle,
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class StoredApp:
    """A fetched App: its catalog summary + the parsed envelope dict (``GetApp``)."""

    summary: AppSummary
    envelope: Dict[str, Any]
    # 32-byte HANDLE-FREE App identity as hex (blake3 over the canonical envelope):
    # identical for byte-identical envelopes regardless of the handle they are stored
    # under (contrast ``summary.app_ref``, which is handle-scoped). "" when not found.
    app_digest: str
    # OPTIONAL 64-hex lineage hint — the ``app_digest`` this App was imported/cloned
    # from ("" ⇒ authored-here). Off-identity; a provenance hint, never authenticity.
    source_digest: str = ""

    @classmethod
    def from_proto(cls, r: "_g.GetAppResponse") -> "StoredApp":
        envelope = json.loads(bytes(r.envelope_json).decode("utf-8")) if r.envelope_json else {}
        return cls(
            summary=AppSummary.from_proto(r.summary),
            envelope=envelope,
            app_digest=hexids.encode(r.app_digest),
            source_digest=hexids.encode(r.source_digest),
        )


@dataclass
class Skill:
    """A named (instructions + tool SET) bundle ≈ a reusable Agent (POC-4 minimal
    artifact rail). Provide ``instructions`` (a body uploaded at ``save``) OR an
    ``instructions_ref`` (a 64-hex content ref already in the store)."""

    name: str
    instructions: str = ""
    instructions_ref: str = ""
    tools: Mapping[str, str] = field(default_factory=dict)
