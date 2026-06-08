/**
 * The teams (membership) views — a team, its members + roles, and a member's
 * optional resolved warrant on an asset, as enumerated by `ListTeams` /
 * `ListTeamMembers` (UI-3). Kept in its own module so `types.ts` stays a thin
 * aggregator, mirroring the Rust core's module-per-concern discipline.
 *
 * Every field is a server-rendered DISPLAY projection — never the warrant body or
 * any secret. The OSS surface is VIEW-only; managing teams across parties is cloud.
 */

import type {
  ListTeamMembersResponse as PbListTeamMembersResponse,
  ListTeamsResponse as PbListTeamsResponse,
  TeamMember as PbTeamMember,
  TeamSummary as PbTeamSummary,
  WarrantView as PbWarrantView,
} from "./gen/kortecx/v1/gateway_pb.js";

/**
 * A compact, human-readable warrant projection — the headline ceilings + scopes a
 * member's resolved warrant conveys, as display strings/scalars. NEVER the warrant
 * body/secret.
 */
export class WarrantView {
  constructor(
    readonly executorClass: string,
    readonly modelRoute: string,
    readonly netScope: string,
    readonly fsScope: string,
    readonly maxCalls: number,
    readonly cpuMilli: number,
    readonly wallClockMs: number,
  ) {}

  static fromProto(w: PbWarrantView): WarrantView {
    return new WarrantView(
      w.executorClass,
      w.modelRoute,
      w.netScope,
      w.fsScope,
      Number(w.maxCalls),
      Number(w.cpuMilli),
      Number(w.wallClockMs),
    );
  }
}

/** One team in a `ListTeams` enumeration. */
export class TeamSummary {
  constructor(
    readonly teamId: string,
    readonly displayName: string,
    readonly owner: string,
    readonly memberCount: number,
  ) {}

  static fromProto(t: PbTeamSummary): TeamSummary {
    return new TeamSummary(t.teamId, t.displayName, t.owner, t.memberCount);
  }
}

/** One member of a team, with the optional resolved-warrant projection. */
export class TeamMember {
  constructor(
    readonly party: string,
    readonly role: string,
    readonly actionCaps: readonly string[],
    /** Present iff `listTeamMembers` was called with an `assetRef` and a path resolves. */
    readonly resolvedWarrant: WarrantView | null,
  ) {}

  static fromProto(m: PbTeamMember): TeamMember {
    return new TeamMember(
      m.party,
      m.role,
      m.actionCaps,
      m.resolvedWarrant ? WarrantView.fromProto(m.resolvedWarrant) : null,
    );
  }

  /** `true` iff this member's cap conveys catalog `Delegate` (a team delegate). */
  get isDelegate(): boolean {
    return this.actionCaps.includes("Delegate");
  }
}

/** A team's members (with the owner echoed so a viewer can mark the owner row). */
export class TeamMembers {
  constructor(
    readonly owner: string,
    readonly members: readonly TeamMember[],
  ) {}

  static fromProto(r: PbListTeamMembersResponse): TeamMembers {
    return new TeamMembers(
      r.owner,
      r.members.map((m) => TeamMember.fromProto(m)),
    );
  }
}

/** Map a `ListTeams` response to the view list. */
export function teamsFromProto(r: PbListTeamsResponse): TeamSummary[] {
  return r.teams.map((t) => TeamSummary.fromProto(t));
}
