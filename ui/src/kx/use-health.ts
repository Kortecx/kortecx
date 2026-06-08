/**
 * Gateway liveness, derived from a cheap unary round-trip (`listSignatures`) on an
 * interval. The browser CANNOT reach the separate `grpc.health.v1` service through
 * the grpc-web shim, so liveness is inferred from a real gRPC answer — exactly the
 * probe `connect()` uses: a reachable-but-UNIMPLEMENTED gateway is still "live".
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export type Health = "live" | "degraded" | "down";

export function useHealth(intervalMs = 5000) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.health(endpoint),
    enabled: status === "connected" && client !== null,
    refetchInterval: intervalMs,
    refetchIntervalInBackground: false,
    queryFn: async (): Promise<Health> => {
      if (!client) {
        return "down";
      }
      try {
        await client.listSignatures();
        return "live";
      } catch (e) {
        const ui = toUiError(e);
        // Reachable but the read path isn't wired → still live (same as connect()).
        if (ui.kind === "not-wired") {
          return "live";
        }
        // Transport-level unreachability → down; anything else reachable → degraded.
        return ui.kind === "retry" ? "down" : "degraded";
      }
    },
  });
}
