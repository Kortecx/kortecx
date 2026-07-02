"""RC-SW1 skill-catalog views — declarative ``kortecx.skill/v1`` bundles.

A skill is instructions + a tool grant-WISH set; adding one grants NOTHING. At
``run_app`` the server intersects the wish against the caller's grants and the
live broker (``wish ∩ grants ∩ fireable``). SN-8: ``skill_ref`` and
``instructions_ref`` are SERVER-DERIVED — the client sends bytes, never an
identity. The catalog is an off-journal ``skills.db`` sidecar
(rebuildable-to-empty), caller-scoped, with UNIFORM not-found.

Kept in its own module (the Rust core's module-per-concern discipline, GR3).
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Dict, List

from . import hexids

if TYPE_CHECKING:  # pragma: no cover - typing only
    from .v1 import gateway_pb2 as _g

SKILL_SCHEMA = "kortecx.skill/v1"
"""The manifest schema/version tag — readers fail closed on a mismatch."""


@dataclass(frozen=True)
class SkillSummary:
    """A stored skill's catalog/display view (manifest-derived + server id)."""

    skill_ref: str  # server-derived canonical-manifest hash, as hex
    name: str
    version: str
    description: str
    instructions_ref: str  # 64-hex content-store ref to the instructions body
    tools: Dict[str, str]  # the tool grant-WISH set (a wish, never a grant)
    tags: List[str]

    @classmethod
    def from_proto(cls, s: "_g.SkillSummary") -> "SkillSummary":
        return cls(
            skill_ref=hexids.encode(s.skill_ref),
            name=s.name,
            version=s.version,
            description=s.description,
            instructions_ref=s.instructions_ref,
            tools=dict(s.tools),
            tags=list(s.tags),
        )


@dataclass(frozen=True)
class AddSkillResult:
    """The outcome of an ``AddSkill`` upsert (server-derived refs + dedup signal)."""

    skill_ref: str
    name: str
    instructions_ref: str
    deduplicated: bool

    @classmethod
    def from_proto(cls, r: "_g.AddSkillResponse") -> "AddSkillResult":
        return cls(
            skill_ref=hexids.encode(r.skill_ref),
            name=r.name,
            instructions_ref=r.instructions_ref,
            deduplicated=r.deduplicated,
        )


@dataclass(frozen=True)
class SkillWish:
    """One wished tool with the ADVISORY ``registered`` bit (display only)."""

    tool_id: str
    tool_version: str
    registered: bool  # could THIS serve currently fire it? never a grant.


@dataclass(frozen=True)
class SkillForm:
    """The ``GetSkillForm`` view: summary + wishes + the instructions preview."""

    summary: SkillSummary
    wishes: List[SkillWish]
    instructions_preview: str  # server-capped excerpt ('' when added by ref)
    preview_truncated: bool

    @classmethod
    def from_proto(cls, r: "_g.GetSkillFormResponse") -> "SkillForm":
        return cls(
            summary=SkillSummary.from_proto(r.summary),
            wishes=[
                SkillWish(
                    tool_id=w.tool_id,
                    tool_version=w.tool_version,
                    registered=w.registered,
                )
                for w in r.wishes
            ],
            instructions_preview=r.instructions_preview,
            preview_truncated=r.preview_truncated,
        )
