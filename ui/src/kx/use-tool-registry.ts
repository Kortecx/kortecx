/**
 * The durable tools-registry hooks (PR-6a-1): the inventory view (`DiscoverTools`)
 * plus the operator register/deregister mutations (`RegisterTool`/`DeregisterTool`).
 *
 * DISTINCT from the advisory toolscout view (`use-toolscout.ts`): this is the
 * durable GOVERNANCE surface — what is registered, by whom, with what authority.
 * Registration grants NO authority (SN-8); the `toolId` is SERVER-derived. DIALING
 * a registered external MCP server is a Cloud / PR-6b capability — registering a
 * host only records it (SSRF-vetted at admission). Degrades to a not-wired empty
 * state on a gateway without the registry (UNIMPLEMENTED).
 */

import type { RegisterToolInput, RegisteredToolsPage } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useDiscoverTools() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.discoverTools(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<RegisteredToolsPage> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.discoverTools({});
    },
  });
  return {
    tools: q.data?.tools ?? [],
    hasMore: q.data?.hasMore ?? false,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useRegisterTool() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<string, unknown, RegisterToolInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.registerTool(input);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}

export function useDeregisterTool() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { name: string; version: string }>({
    mutationFn: async ({ name, version }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deregisterTool(name, version);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}
