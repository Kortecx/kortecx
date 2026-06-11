/**
 * The toolscout viewers: the gateway's advisory tool manifests (`ListToolManifests`)
 * and the TaskBundle dry-run scorer (`ScoreTaskBundle`). Both tolerate a gateway
 * that has not wired the toolscout view (UNIMPLEMENTED → the query errors; the
 * Tools view degrades to a not-wired empty state). SN-8: every score/verdict is
 * ADVISORY/DISPLAY-ONLY — a score can surface a tool, never grant one.
 */

import type { BundleScore, BundleSpec, ToolManifest } from "@kortecx/sdk/web";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useToolManifests() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.toolManifests(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<ToolManifest[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listToolManifests();
    },
  });
}

export function useScoreBundle() {
  const { client } = useConnection();
  return useMutation<BundleScore, unknown, BundleSpec>({
    mutationFn: async (spec) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.scoreTaskBundle(spec);
    },
  });
}
