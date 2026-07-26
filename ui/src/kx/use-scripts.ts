/**
 * The durable script-registry hooks — the inventory view (`ListScripts`) plus the
 * operator register/deregister mutations.
 *
 * A script is a named, versioned program the operator registers once and the
 * runtime fires as an ordinary tool: the same registry, the same
 * `(name, version)` grant key, the same broker. It appears under its own tab
 * because its DECLARATION differs — source, an interpreter, a resource wish —
 * not because its authority does.
 *
 * Registration grants NO authority. The declared wish becomes the tool's
 * requirement and the runtime still refuses any call whose requirement is not a
 * subset of the calling warrant, so what is shown here is what a script ASKED
 * for, never what it may do. Degrades to a not-wired empty state on a gateway
 * without scripts (UNIMPLEMENTED).
 */

import type { RegisterScriptInput, RegisteredScriptsPage } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useListScripts() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.listScripts(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<RegisteredScriptsPage> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listScripts({});
    },
  });
  return {
    scripts: q.data?.scripts ?? [],
    hasMore: q.data?.hasMore ?? false,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useRegisterScript() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<string, unknown, RegisterScriptInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.registerScript(input);
    },
    onSuccess: () => {
      // A script IS a tool, so the tool inventory changes too — refreshing only
      // the script list would leave the Tools tab showing a stale registry.
      void qc.invalidateQueries({ queryKey: queryKeys.listScripts(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}

export function useDeregisterScript() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { name: string; version: string }>({
    mutationFn: async ({ name, version }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deregisterScript(name, version);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.listScripts(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}
