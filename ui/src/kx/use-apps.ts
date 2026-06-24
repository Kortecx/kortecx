/**
 * The POC-4 App-catalog hooks — the read-only inventory (`ListApps`), one App's
 * full envelope (`GetApp`), and the `runApp` mutation (client-compose over
 * `GetApp` → `submitWorkflow`).
 *
 * An App is a durable, reusable `kortecx.app/v1` envelope (a portable blueprint
 * wrapped with by-reference references + a 4-axis steering config). SN-8: `appRef`
 * is SERVER-derived; Apps are caller-scoped (a not-found / not-owned App is uniform
 * — no cross-party existence oracle). The envelope carries NO authority — `runApp`
 * re-compiles the blueprint and the server re-resolves every warrant from the
 * caller's grants. Degrades to a not-wired empty state on an old gateway.
 */

import type { AppSummary, StoredApp } from "@kortecx/sdk/web";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useApps() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.apps(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<AppSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listApps();
    },
  });
  return {
    apps: q.data ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useApp(handle: string | null) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.app(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && handle !== null,
    queryFn: async (): Promise<StoredApp | null> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getApp(handle);
    },
  });
}

export interface RunAppResult {
  readonly instanceId: string;
}

export function useRunApp() {
  const { client } = useConnection();
  return useMutation<RunAppResult, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      // No `wait` ⇒ a Run handle (its ids are already hex) — route to the live run.
      const run = await client.runApp(handle);
      if (!("recipeFingerprint" in run)) {
        throw new Error("unexpected runApp result");
      }
      return { instanceId: run.instanceId };
    },
  });
}
