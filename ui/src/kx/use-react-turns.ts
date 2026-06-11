/**
 * The ReAct-turn history hook (`ListReactTurns`) — the durable Reason→Act→Observe
 * trail of a gateway's live ReAct chains. Read-only, newest-first; optionally scoped
 * to one run. Degrades to an empty list with `notWired` when the RPC is unwired.
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useReactTurns(opts: { instanceId?: string; limit?: number } = {}) {
  const { client, endpoint, status } = useConnection();
  const limit = opts.limit ?? 100;
  const instanceId = opts.instanceId;
  const q = useQuery({
    queryKey: queryKeys.reactTurns(endpoint, instanceId, limit),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listReactTurns({ ...(instanceId ? { instanceId } : {}), limit });
    },
  });
  return {
    turns: q.data?.turns ?? [],
    hasMore: q.data?.hasMore ?? false,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    refetch: q.refetch,
  };
}
