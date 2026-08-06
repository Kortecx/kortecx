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
import { chainAnchors } from "../lib/react-chain-anchors";
import {
  type ChainsByInstance,
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
/** One page of turn rows across every chain on the node — the server clamps this. */
const CHAIN_PAGE = 500;

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

  // The agentic chains on this node, so a run started OUTSIDE this browser (`kx agent
  // run`, `kx chat --tools`, a trigger) can still be opened as itself. Listing turns
  // with no chain key returns every chain, and each row names the one it belongs to.
  // Fail-soft: an older gateway, or a chain aged out of the page, simply yields no
  // anchors and those runs stay unscoped — never a fabricated one.
  const chainsQuery = useQuery({
    queryKey: queryKeys.reactTurns(endpoint, undefined, CHAIN_PAGE, undefined),
    enabled: status === "connected" && client !== null,
    retry: false,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listReactTurns({ limit: CHAIN_PAGE });
    },
  });

  const notWired = server.isError && toUiError(server.error).kind === "not-wired";
  const serverRuns = server.data?.runs ?? [];
  const chainRows = chainsQuery.data?.turns;
  const chains = useMemo<ChainsByInstance>(() => {
    const rowsByInstance = new Map<string, (typeof chainRows & object)[number][]>();
    for (const row of chainRows ?? []) {
      const bucket = rowsByInstance.get(row.instanceId);
      if (bucket === undefined) {
        rowsByInstance.set(row.instanceId, [row]);
      } else {
        bucket.push(row);
      }
    }
    return new Map(
      [...rowsByInstance].map(([instanceId, rows]) => [instanceId, chainAnchors(rows)]),
    );
  }, [chainRows]);
  const runs = useMemo(
    () => mergeServerRuns(local, serverRuns, chains),
    [local, serverRuns, chains],
  );

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
