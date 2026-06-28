/**
 * The per-run quality hook (`ScoreRun`, RC1/D172) — an EXPECTATION-FREE summary of one
 * run's trajectory (terminal reached, turns / tool-calls spent, budget burn, rejection
 * count). Enabled only once a run id is supplied; degrades to `notWired` when the RPC is
 * absent on the connected gateway. The golden-suite gate runs offline (`kx eval run`).
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useEvalScore(instanceId: string | undefined) {
  const { client, endpoint, status } = useConnection();
  const id = instanceId?.trim();
  const q = useQuery({
    queryKey: queryKeys.evalScore(endpoint, id ?? ""),
    enabled: status === "connected" && client !== null && !!id,
    queryFn: async () => {
      if (!client || !id) {
        throw new Error("not connected");
      }
      return client.scoreRun(id);
    },
  });
  return {
    score: q.data ?? null,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    error: q.isError ? toUiError(q.error) : null,
    isLoading: q.isLoading,
    refetch: q.refetch,
  };
}
