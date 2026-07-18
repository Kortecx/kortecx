/**
 * D213 Experience lane — hosted-app supervisor hooks. Start (or attach to) a hosted
 * app's dev server (`StartHostedApp`) and poll its live status (`GetHostedAppStatus`).
 * `startHostedApp` returns immediately with the loopback URL once running; the caller
 * opens it in a new browser tab. Degrades to a not-wired signal on a gateway built
 * without the `hosted-apps` feature (the console hides the Run control).
 */

import type { HostedAppStatus } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** Start (or attach to) a hosted app's dev server; resolves with its live status. */
export function useStartHostedApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<HostedAppStatus, unknown, { handle: string; rebuild?: boolean }>({
    mutationFn: async ({ handle, rebuild }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.startHostedApp(handle, rebuild ? { rebuild } : {});
    },
    onSuccess: (_status, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.hostedAppStatus(endpoint, handle) });
    },
  });
}

/** Stop a hosted app's dev server. */
export function useStopHostedApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.stopHostedApp(handle);
    },
    onSuccess: (_ok, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.hostedAppStatus(endpoint, handle) });
    },
  });
}

/** Poll a hosted app's status; polls while starting/running, stops once stopped/failed. */
export function useHostedAppStatus(handle: string | null, enabled: boolean) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.hostedAppStatus(endpoint, handle ?? ""),
    enabled: enabled && status === "connected" && client !== null && handle !== null,
    refetchInterval: (query) => {
      const s = (query.state.data as HostedAppStatus | undefined)?.state;
      return s === "running" || s === "starting" || s === "installing" || s === "materializing"
        ? 3000
        : false;
    },
    queryFn: async (): Promise<HostedAppStatus> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getHostedAppStatus(handle);
    },
  });
  return {
    status: q.data ?? null,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isError: q.isError,
    refetch: q.refetch,
  };
}
