"""UI-3 teams (membership) views — a team, its members + roles, and a member's
optional resolved warrant on an asset (``ListTeams`` / ``ListTeamMembers``).

Kept in its own module so ``types.py`` stays a thin aggregator. Every field is a
server-rendered DISPLAY projection — never the warrant body or any secret. The OSS
surface is VIEW-only; managing teams across parties is cloud.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import List, Optional

from .v1 import gateway_pb2 as _g


@dataclass(frozen=True)
class WarrantView:
    """A compact, human-readable warrant projection — the headline ceilings + scopes
    a member's resolved warrant conveys. NEVER the warrant body/secret."""

    executor_class: str
    model_route: str
    net_scope: str
    fs_scope: str
    max_calls: int
    cpu_milli: int
    wall_clock_ms: int

    @classmethod
    def from_proto(cls, w: "_g.WarrantView") -> "WarrantView":
        return cls(
            executor_class=w.executor_class,
            model_route=w.model_route,
            net_scope=w.net_scope,
            fs_scope=w.fs_scope,
            max_calls=w.max_calls,
            cpu_milli=w.cpu_milli,
            wall_clock_ms=w.wall_clock_ms,
        )


@dataclass(frozen=True)
class TeamSummary:
    """One team in a ``ListTeams`` enumeration."""

    team_id: str
    display_name: str
    owner: str
    member_count: int

    @classmethod
    def from_proto(cls, t: "_g.TeamSummary") -> "TeamSummary":
        return cls(
            team_id=t.team_id,
            display_name=t.display_name,
            owner=t.owner,
            member_count=t.member_count,
        )


@dataclass(frozen=True)
class TeamMember:
    """One member of a team, with the optional resolved-warrant projection."""

    party: str
    role: str
    action_caps: List[str]
    resolved_warrant: Optional[WarrantView]

    @classmethod
    def from_proto(cls, m: "_g.TeamMember") -> "TeamMember":
        return cls(
            party=m.party,
            role=m.role,
            action_caps=list(m.action_caps),
            resolved_warrant=(
                WarrantView.from_proto(m.resolved_warrant)
                if m.HasField("resolved_warrant")
                else None
            ),
        )

    @property
    def is_delegate(self) -> bool:
        """``True`` iff this member's cap conveys catalog ``Delegate``."""
        return "Delegate" in self.action_caps


@dataclass(frozen=True)
class TeamMembers:
    """A team's members (with the owner echoed so a viewer can mark the owner row)."""

    owner: str
    members: List[TeamMember]

    @classmethod
    def from_proto(cls, r: "_g.ListTeamMembersResponse") -> "TeamMembers":
        return cls(
            owner=r.owner,
            members=[TeamMember.from_proto(m) for m in r.members],
        )
