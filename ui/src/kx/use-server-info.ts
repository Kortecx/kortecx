/**
 * POC-1 Settings "Workspace": the non-secret server configuration (`GetServerInfo`)
 * — the resolved model, endpoints, durable paths, limits, the auth/TLS POSTURE, and
 * the build's feature flags. Governed by an authenticated caller; the response never
 * carries a secret. A gateway that predates the RPC (or an unauthenticated caller)
 * surfaces the error so the Settings card degrades honestly rather than faking facts.
 */

import type { ServerInfo } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useServerInfo() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.serverInfo(endpoint),
    enabled: status === "connected" && client !== null,
    retry: false,
    queryFn: async (): Promise<ServerInfo> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.getServerInfo();
    },
  });
}
