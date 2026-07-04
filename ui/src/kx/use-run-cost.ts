/**
 * RC6a cost-readout hook (M11) — a run's DISPLAY-ONLY local spend estimate over the
 * durable turn/tool counters at operator-set micro-USD rates (`GetRunCost`). A
 * budget guardrail readout, NOT Cloud per-expert billing (the D129/GR19 boundary
 * holds). Degrades to a not-wired empty state on a gateway without the cost admin
 * (UNIMPLEMENTED). Fetched on demand for one run (no polling — the counters only
 * move while the run is live, and the run detail already refreshes).
 */

import type { RunCost } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** One run's spend estimate (`GetRunCost`), keyed by instance id (hex). */
export function useRunCost(instanceId: string | undefined) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.runCost(endpoint, instanceId ?? ""),
    enabled: status === "connected" && client !== null && !!instanceId,
    queryFn: async (): Promise<RunCost | null> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      return client.cost.getRunCost(instanceId);
    },
  });
  return {
    cost: q.data ?? null,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.isError && toUiError(q.error).kind !== "not-wired" ? toUiError(q.error) : null,
    refetch: q.refetch,
  };
}
