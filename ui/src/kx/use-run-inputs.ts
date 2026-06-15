/**
 * `GetRunInputs` (PR-D "Re-run with changes"): the args a run was submitted with,
 * fetched so a run recovered from `ListRuns` (no client-side history) can pre-fill
 * its recipe form and be re-invoked with edits. A run with nothing captured
 * (pre-PR-D / rebuilt-to-empty sidecar) is `NotFound`; an old gateway without the
 * sidecar is `Unimplemented` — both surface as the `not-wired`/`not-found` UI error
 * so the caller can degrade to a blank form honestly (don't-fake-gaps).
 */

import type { RunInputs } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useRunInputs(instanceId: string | undefined, enabled = true) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.runInputs(endpoint, instanceId ?? ""),
    enabled: enabled && status === "connected" && client !== null && Boolean(instanceId),
    staleTime: Number.POSITIVE_INFINITY, // a run's captured args are immutable for that instance
    retry: false, // NotFound/Unimplemented are terminal — don't spin
    queryFn: async (): Promise<RunInputs> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      return client.getRunInputs(instanceId);
    },
  });
}
