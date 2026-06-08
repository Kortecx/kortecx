/**
 * The teams (membership) viewers: the teams a gateway knows (`ListTeams`) and a
 * team's members + roles, with an optional per-member resolved warrant on an asset
 * (`ListTeamMembers`). Both tolerate a gateway that has not wired the membership
 * view (UNIMPLEMENTED → the query errors; the Systems view degrades to a not-wired
 * empty state). VIEW-only in OSS — managing teams across parties is cloud.
 */

import type { TeamMembers, TeamSummary } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useTeams() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.teams(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<TeamSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listTeams();
    },
  });
}

export function useTeamMembers(teamId: string | undefined, assetRef?: string) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.teamMembers(endpoint, teamId ?? "", assetRef),
    enabled: status === "connected" && client !== null && Boolean(teamId),
    queryFn: async (): Promise<TeamMembers> => {
      if (!client || !teamId) {
        throw new Error("not connected");
      }
      return client.listTeamMembers(teamId, assetRef ? { assetRef } : {});
    },
  });
}
