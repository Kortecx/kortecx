"""UI-3 team + grant views — pure unit tests (no server)."""

from __future__ import annotations

from kortecx import AssetGrants, GrantView, TeamMember, TeamMembers, TeamSummary
from kortecx.v1 import gateway_pb2 as g


def test_team_summary_from_proto():
    t = g.TeamSummary(
        team_id="kx/teams/workspace", display_name="Workspace", owner="kx-gateway", member_count=3
    )
    s = TeamSummary.from_proto(t)
    assert s.team_id == "kx/teams/workspace"
    assert s.owner == "kx-gateway"
    assert s.member_count == 3


def test_team_members_flag_delegate_and_resolved_warrant():
    r = g.ListTeamMembersResponse(
        owner="kx-gateway",
        members=[
            g.TeamMember(
                party="alice@acme",
                role="demo-delegate",
                action_caps=["Read", "Use", "Delegate"],
                resolved_warrant=g.WarrantView(
                    executor_class="Bwrap", model_route="m ×3", max_calls=3, wall_clock_ms=30000
                ),
            ),
            g.TeamMember(party="bob@acme", role="demo-member", action_caps=["Read", "Use"]),
        ],
    )
    m = TeamMembers.from_proto(r)
    assert m.owner == "kx-gateway"
    assert isinstance(m.members[0], TeamMember)
    assert m.members[0].is_delegate is True
    assert m.members[0].resolved_warrant is not None
    assert m.members[0].resolved_warrant.max_calls == 3
    assert m.members[0].resolved_warrant.wall_clock_ms == 30000
    # bob: not a delegate; no resolved warrant (no asset_ref was supplied).
    assert m.members[1].is_delegate is False
    assert m.members[1].resolved_warrant is None


def test_asset_grants_classify_root_delegated_revoked():
    r = g.ListAssetGrantsResponse(
        owner="kx-gateway",
        grants=[
            g.GrantView(
                grantor="kx-gateway",
                grantee="kx/teams/workspace",
                actions=["Read", "Use"],
                runtime_scope="demo",
                is_root=True,
                revoked=False,
            ),
            g.GrantView(
                grantor="alice@acme",
                grantee="bob@acme",
                actions=["Use"],
                runtime_scope="demo",
                is_root=False,
                revoked=True,
            ),
        ],
    )
    ag = AssetGrants.from_proto(r)
    assert ag.owner == "kx-gateway"
    assert isinstance(ag.grants[0], GrantView)
    assert ag.grants[0].status == "root"
    assert ag.grants[1].status == "revoked"


def test_empty_views_are_valid():
    assert TeamMembers.from_proto(g.ListTeamMembersResponse(owner="o")).members == []
    assert AssetGrants.from_proto(g.ListAssetGrantsResponse(owner="")).grants == []
