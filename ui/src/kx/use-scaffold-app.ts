/**
 * POC-5a App-scaffold hooks — launch the server-side agentic scaffold
 * (`ScaffoldApp`) and POLL its live status (`GetScaffoldStatus`).
 *
 * The scaffold writes a FIXED skeleton project tree into the App's CoW branch
 * (the host is never written; SN-8: the branch is caller-scoped, the App handle
 * IS the project branch handle). `scaffoldApp` returns immediately; the status
 * query drives the honest per-file progress UI (GR15 — real `filesDone` /
 * `filesPending`, never a timer). Polling stops the moment the server reports
 * `done` / `failed`; on `done` we invalidate the Apps + branch caches so the new
 * tree appears.
 */

import type { ScaffoldStatus } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export interface ScaffoldVars {
  readonly handle: string;
  readonly goal?: string;
}

export interface ScaffoldLaunch {
  readonly branchHandle: string;
  readonly resumed: boolean;
}

/** Launch (or resume) the agentic scaffold for an App. Returns the project
 *  branch handle to poll (the App's own handle, by convention). */
export function useScaffoldApp() {
  const { client } = useConnection();
  return useMutation<ScaffoldLaunch, unknown, ScaffoldVars>({
    mutationFn: async ({ handle, goal }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.scaffoldApp(handle, goal !== undefined ? { goal } : {});
    },
  });
}

/** Short-poll the scaffold status while it's active; the caller stops by passing
 *  `enabled={false}` once the phase is terminal (done/failed). */
export function useScaffoldStatus(branchHandle: string | null, enabled: boolean) {
  const { client, endpoint, status } = useConnection();
  const query = useQuery({
    queryKey: queryKeys.scaffoldStatus(endpoint, branchHandle ?? ""),
    enabled: enabled && status === "connected" && client !== null && branchHandle !== null,
    // Poll while active; the status SHAPE drives the stop (the consumer disables
    // `enabled` on a terminal phase, and refetchInterval halts on disabled).
    refetchInterval: (q) => {
      const data = q.state.data as ScaffoldStatus | undefined;
      if (data && (data.phase === "done" || data.phase === "failed")) {
        return false;
      }
      return 900;
    },
    queryFn: async (): Promise<ScaffoldStatus> => {
      if (!client || branchHandle === null) {
        throw new Error("not connected");
      }
      return client.getScaffoldStatus(branchHandle);
    },
  });
  return query;
}

/** Invalidate the Apps catalog + branch inventory + this App's branch manifest
 *  once a scaffold completes (the new project tree is now committed). */
export function useInvalidateOnScaffoldDone() {
  const { endpoint } = useConnection();
  const qc = useQueryClient();
  return (branchHandle: string) => {
    void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
    void qc.invalidateQueries({ queryKey: queryKeys.branches(endpoint) });
    void qc.invalidateQueries({ queryKey: queryKeys.appBranch(endpoint, branchHandle) });
  };
}
