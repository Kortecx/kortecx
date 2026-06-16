/**
 * The operator alerts inbox pages (`ListAlerts`, W1a-2) — the journal's TERMINAL
 * `Failed` facts (dead-letters + worker-reported terminal failures) folded
 * newest-first into the gateway's rebuildable-to-empty `alerts.db` read-cache.
 * Cursor-paged with `before_seq` through `useInfiniteQuery` (the rows are
 * append-only and seq-keyed, so pages never shift under the cursor). Degrades to
 * `notWired` when the RPC is unimplemented (an older gateway or a sidecar-less
 * serve) — the telemetry convention.
 *
 * Read-only by construction: the triage lifecycle (acknowledge/resolve), the
 * alert-rule engine, and notifications are a Cloud capability (D156/D129) — this
 * hook exposes no mutation.
 */

import { useInfiniteQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

const PAGE = 100;

export function useAlerts(opts: { instanceId?: string; pageSize?: number } = {}) {
  const { client, endpoint, status } = useConnection();
  const pageSize = opts.pageSize ?? PAGE;
  const instanceId = opts.instanceId;
  const q = useInfiniteQuery({
    queryKey: queryKeys.alerts(endpoint, instanceId, pageSize),
    enabled: status === "connected" && client !== null,
    initialPageParam: undefined as number | undefined,
    queryFn: async ({ pageParam }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listAlerts({
        ...(instanceId ? { instanceId } : {}),
        ...(pageParam != null ? { beforeSeq: BigInt(pageParam) } : {}),
        limit: pageSize,
      });
    },
    getNextPageParam: (last) => {
      if (!last.hasMore || last.alerts.length === 0) {
        return undefined;
      }
      return last.alerts[last.alerts.length - 1]?.seq;
    },
  });
  return {
    alerts: q.data?.pages.flatMap((p) => p.alerts) ?? [],
    hasMore: q.hasNextPage,
    loadMore: () => void q.fetchNextPage(),
    isLoadingMore: q.isFetchingNextPage,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    error: q.isError && toUiError(q.error).kind !== "not-wired" ? toUiError(q.error) : null,
    refetch: () => void q.refetch(),
  };
}
