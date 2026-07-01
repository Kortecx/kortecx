/**
 * The re-rank-turn history hook (`ListReRankTurns`) — the durable trail of a
 * gateway's live listwise LLM re-rank loops (RC4c-2). Read-only, newest-first.
 * Degrades to an empty list with `notWired` when the gateway does not wire the RPC
 * (older binary).
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useRerankTurns(limit = 100) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.rerankTurns(endpoint, limit),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listRerankTurns({ limit });
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
