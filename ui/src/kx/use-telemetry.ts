/**
 * The mote execution-telemetry pages (`ListMoteTelemetry`, Batch C) — the
 * host-measured exhaust rows (wall-clock / model usage / fired tool), newest
 * first. Cursor-paged with `before_seq` through `useInfiniteQuery` (the rows
 * are append-only and seq-keyed, so pages never shift under the cursor).
 * Degrades to `notWired` when the RPC is unimplemented (an older gateway or a
 * sidecar-less serve) — the capture-records convention.
 */

import { useInfiniteQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

const PAGE = 100;

export function useTelemetry(opts: { instanceId?: string; pageSize?: number } = {}) {
  const { client, endpoint, status } = useConnection();
  const pageSize = opts.pageSize ?? PAGE;
  const instanceId = opts.instanceId;
  const q = useInfiniteQuery({
    queryKey: queryKeys.telemetry(endpoint, instanceId, pageSize),
    enabled: status === "connected" && client !== null,
    initialPageParam: undefined as number | undefined,
    queryFn: async ({ pageParam }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listMoteTelemetry({
        ...(instanceId ? { instanceId } : {}),
        ...(pageParam != null ? { beforeSeq: BigInt(pageParam) } : {}),
        limit: pageSize,
      });
    },
    getNextPageParam: (last) => {
      if (!last.hasMore || last.rows.length === 0) {
        return undefined;
      }
      return last.rows[last.rows.length - 1]?.seq;
    },
  });
  return {
    rows: q.data?.pages.flatMap((p) => p.rows) ?? [],
    hasMore: q.hasNextPage,
    loadMore: () => void q.fetchNextPage(),
    isLoadingMore: q.isFetchingNextPage,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    error: q.isError && toUiError(q.error).kind !== "not-wired" ? toUiError(q.error) : null,
    refetch: () => void q.refetch(),
  };
}
