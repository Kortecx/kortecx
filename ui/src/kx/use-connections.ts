/**
 * The external MCP gateway hooks (PR-6b-1): the live Connections govern surface —
 * list registered servers (`ListMcpServers`), register + DIAL a server
 * (`RegisterMcpServer`), test reachability (`TestMcpServer`), re-discover
 * (`DiscoverServerTools`), and remove (`DeregisterMcpServer`).
 *
 * The runtime is a SECURE GATEWAY (D132/D159/GR19): registering a server DIALS it
 * (the live untrusted-egress surface — host SSRF-vetted at admission AND at dial).
 * Server ids are server-derived (SN-8); a credential is referenced by NAME only
 * (never the secret, D81). Degrades to a not-wired empty state on a gateway
 * without the MCP gateway feature (UNIMPLEMENTED).
 */

import type {
  McpServersPage,
  RegisterMcpServerInput,
  RegisterServerResult,
  RegisteredToolsPage,
} from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useListMcpServers() {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.mcpServers(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<McpServersPage> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listMcpServers({});
    },
  });
  return {
    servers: q.data?.servers ?? [],
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    isError: q.isError,
    error: q.error,
    refetch: q.refetch,
  };
}

export function useRegisterMcpServer() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<RegisterServerResult, unknown, RegisterMcpServerInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.registerMcpServer(input);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.mcpServers(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}

export function useTestMcpServer() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.testMcpServer(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.mcpServers(endpoint) });
    },
  });
}

export function useDiscoverServerTools() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<RegisteredToolsPage, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.discoverServerTools(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.mcpServers(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}

export function useDeregisterMcpServer() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, string>({
    mutationFn: async (name) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deregisterMcpServer(name);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.mcpServers(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.discoverTools(endpoint) });
    },
  });
}
