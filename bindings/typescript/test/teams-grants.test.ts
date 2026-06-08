/** UI-3 team + grant views — pure, no server. */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  GrantViewSchema,
  ListAssetGrantsResponseSchema,
  ListTeamMembersResponseSchema,
  ListTeamsResponseSchema,
  TeamMemberSchema,
  TeamSummarySchema,
  WarrantViewSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { AssetGrants, GrantView } from "../src/grants.js";
import { TeamMember, TeamMembers, WarrantView, teamsFromProto } from "../src/teams.js";

describe("teamsFromProto", () => {
  it("maps team summaries", () => {
    const r = create(ListTeamsResponseSchema, {
      teams: [
        create(TeamSummarySchema, {
          teamId: "kx/teams/demo",
          displayName: "Demo Team",
          owner: "kx-gateway",
          memberCount: 3,
        }),
      ],
    });
    const teams = teamsFromProto(r);
    expect(teams).toHaveLength(1);
    expect(teams[0]?.teamId).toBe("kx/teams/demo");
    expect(teams[0]?.owner).toBe("kx-gateway");
    expect(teams[0]?.memberCount).toBe(3);
  });
});

describe("TeamMembers.fromProto", () => {
  it("flags a delegate + carries the resolved warrant (uint64 → number)", () => {
    const r = create(ListTeamMembersResponseSchema, {
      owner: "kx-gateway",
      members: [
        create(TeamMemberSchema, {
          party: "alice@acme",
          role: "demo-delegate",
          actionCaps: ["Read", "Use", "Delegate"],
          resolvedWarrant: create(WarrantViewSchema, {
            executorClass: "Bwrap",
            modelRoute: "m ×3 (4096/512 tok)",
            netScope: "None",
            fsScope: "/tmp/in:ReadOnly",
            maxCalls: 3n,
            cpuMilli: 1000n,
            wallClockMs: 30000n,
          }),
        }),
        create(TeamMemberSchema, {
          party: "bob@acme",
          role: "demo-member",
          actionCaps: ["Read", "Use"],
        }),
      ],
    });
    const m = TeamMembers.fromProto(r);
    expect(m.owner).toBe("kx-gateway");
    expect(m.members[0]).toBeInstanceOf(TeamMember);
    expect(m.members[0]?.isDelegate).toBe(true);
    expect(m.members[0]?.resolvedWarrant).toBeInstanceOf(WarrantView);
    expect(m.members[0]?.resolvedWarrant?.maxCalls).toBe(3);
    expect(m.members[0]?.resolvedWarrant?.wallClockMs).toBe(30000);
    // bob: not a delegate, no resolved warrant.
    expect(m.members[1]?.isDelegate).toBe(false);
    expect(m.members[1]?.resolvedWarrant).toBeNull();
  });
});

describe("AssetGrants.fromProto", () => {
  it("classifies root / delegated / revoked", () => {
    const r = create(ListAssetGrantsResponseSchema, {
      owner: "kx-gateway",
      grants: [
        create(GrantViewSchema, {
          grantor: "kx-gateway",
          grantee: "kx/teams/demo",
          actions: ["Read", "Use"],
          runtimeScope: "demo",
          isRoot: true,
          revoked: false,
        }),
        create(GrantViewSchema, {
          grantor: "alice@acme",
          grantee: "bob@acme",
          actions: ["Use"],
          runtimeScope: "demo",
          isRoot: false,
          revoked: true,
        }),
      ],
    });
    const g = AssetGrants.fromProto(r);
    expect(g.owner).toBe("kx-gateway");
    expect(g.grants[0]).toBeInstanceOf(GrantView);
    expect(g.grants[0]?.status).toBe("root");
    expect(g.grants[1]?.status).toBe("revoked");
  });
});
