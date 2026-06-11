/**
 * The Morphic-capture stream hook (`ListCaptureRecords`) — the durable, join-key-only
 * action exhaust of the serve path. Read-only, newest-first; optionally scoped to one
 * run. Degrades to an empty list with `notWired` when the RPC is unwired.
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useCaptureRecords(opts: { instanceId?: string; limit?: number } = {}) {
  const { client, endpoint, status } = useConnection();
  const limit = opts.limit ?? 100;
  const instanceId = opts.instanceId;
  const q = useQuery({
    queryKey: queryKeys.captureRecords(endpoint, instanceId, limit),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listCaptureRecords({ ...(instanceId ? { instanceId } : {}), limit });
    },
  });
  return {
    records: q.data?.records ?? [],
    hasMore: q.data?.hasMore ?? false,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    refetch: q.refetch,
  };
}
