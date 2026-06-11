/**
 * The run history hook — now backed by the additive `ListRuns` RPC (UI-2), merged
 * with the per-endpoint session history (localStorage).
 *
 * GROUND TRUTH (single-node OSS): the coordinator registers ONE run per journal;
 * every invocation JOINS it (distinct invocations are distinct terminal Motes
 * WITHIN that run). So `ListRuns` enumerates the durable run INSTANCE(s) — the
 * "re-open after losing localStorage" backstop + the cloud multi-run seam — while
 * the localStorage records carry the richer per-invocation handle + terminal Mote.
 * We therefore SHOW both: every local record (keyed by instance), plus any durable
 * instance `ListRuns` returns that the local history doesn't already cover.
 *
 * Forward/backward compatible: a gateway without `ListRuns` (UNIMPLEMENTED) degrades
 * to the localStorage-only view — `serverAvailable` is false and nothing breaks.
 */

import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  RUNS_CHANGED_EVENT,
  type RunRecord,
  clearRuns,
  loadRuns,
  mergeServerRuns,
  recordRun,
} from "../lib/recent-runs";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

const PAGE = 100;

export interface UseRuns {
  /** Local + durable runs, newest-first. */
  readonly runs: RunRecord[];
  /** True once `ListRuns` answered (false while loading or when UNIMPLEMENTED). */
  readonly serverAvailable: boolean;
  /** True when the gateway does not wire `ListRuns` (degraded to local history). */
  readonly notWired: boolean;
  readonly isLoading: boolean;
  /** A further server page exists (cloud multi-run); calls `loadMore` to fetch it. */
  readonly hasMore: boolean;
  add(run: RunRecord): void;
  refresh(): void;
  clear(): void;
  loadMore(): void;
}

export function useRuns(): UseRuns {
  const { client, endpoint, status } = useConnection();
  const [local, setLocal] = useState<RunRecord[]>(() => loadRuns(endpoint));
  const [limit, setLimit] = useState(PAGE);

  // Reload the local history when the gateway changes — never mix two endpoints.
  useEffect(() => {
    setLocal(loadRuns(endpoint));
    setLimit(PAGE);
  }, [endpoint]);

  // Stay fresh across hook INSTANCES in the same tab: another component's
  // `add`/`clear` (e.g. a Blueprints submit while the DevTools dock tails)
  // dispatches RUNS_CHANGED_EVENT — re-read the persisted history.
  useEffect(() => {
    function onRunsChanged(): void {
      setLocal(loadRuns(endpoint));
    }
    window.addEventListener(RUNS_CHANGED_EVENT, onRunsChanged);
    return () => window.removeEventListener(RUNS_CHANGED_EVENT, onRunsChanged);
  }, [endpoint]);

  const server = useQuery({
    queryKey: queryKeys.runs(endpoint, limit),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listRuns({ limit });
    },
  });

  const notWired = server.isError && toUiError(server.error).kind === "not-wired";
  const serverRuns = server.data?.runs ?? [];
  const runs = useMemo(() => mergeServerRuns(local, serverRuns), [local, serverRuns]);

  const add = useCallback((run: RunRecord) => setLocal(recordRun(endpoint, run)), [endpoint]);
  const refresh = useCallback(() => {
    setLocal(loadRuns(endpoint));
    void server.refetch();
  }, [endpoint, server]);
  const clear = useCallback(() => {
    clearRuns(endpoint);
    setLocal([]);
  }, [endpoint]);
  const loadMore = useCallback(() => setLimit((n) => n + PAGE), []);

  return {
    runs,
    serverAvailable: server.isSuccess,
    notWired,
    isLoading: server.isLoading,
    hasMore: server.data?.hasMore ?? false,
    add,
    refresh,
    clear,
    loadMore,
  };
}
