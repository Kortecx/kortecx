/**
 * The re-plan-round history hook (`ListReplanRounds`) — the durable trail of a
 * gateway's model-driven re-plan loops. Read-only, newest-first. Degrades to an
 * empty list with `notWired` when the gateway does not wire the RPC (older binary).
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useReplanRounds(limit = 100) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.replanRounds(endpoint, limit),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listReplanRounds({ limit });
    },
  });
  return {
    rounds: q.data?.rounds ?? [],
    hasMore: q.data?.hasMore ?? false,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    refetch: q.refetch,
  };
}
