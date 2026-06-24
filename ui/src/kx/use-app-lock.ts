/**
 * POC-5b App-lock hooks — lock / unlock an App's project branch (`LockApp` /
 * `UnlockApp`). A locked App REFUSES agentic in-CAS edits at the runtime
 * chokepoint (the agent-write gate); this is the OSS per-App policy surface.
 *
 * By convention the project branch handle IS the App handle (one-App-one-branch),
 * so both mutations key on the App handle. On success we invalidate the Apps
 * catalog (its `locked` flag) and this App's branch + envelope caches so the lock
 * chip + edit gate reflect the new state immediately.
 */

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

function useLockMutation(action: "lock" | "unlock") {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return action === "lock" ? client.lockApp(handle) : client.unlockApp(handle);
    },
    onSuccess: (_data, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.apps(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.app(endpoint, handle) });
      void qc.invalidateQueries({ queryKey: queryKeys.appBranch(endpoint, handle) });
    },
  });
}

export function useLockApp() {
  return useLockMutation("lock");
}

export function useUnlockApp() {
  return useLockMutation("unlock");
}
