/**
 * The exact, cross-page per-model token-economy rollup (`ListTelemetrySummary`,
 * W1a-3) — output tokens + wall-clock summed server-side `GROUP BY model_id`, so
 * a long ReAct run is summed honestly (unlike a client fold over the page-clamped
 * `useTelemetry`). One unary call (no cursor). Token-only: no cost/$ (billing is
 * CLOUD). Degrades to `notWired` when the RPC is unimplemented (an older gateway
 * or a sidecar-less serve) — the `useTelemetry` convention.
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export function useTelemetrySummary(opts: { instanceId?: string } = {}) {
  const { client, endpoint, status } = useConnection();
  const instanceId = opts.instanceId;
  const q = useQuery({
    queryKey: queryKeys.telemetrySummary(endpoint, instanceId),
    enabled: status === "connected" && client !== null,
    queryFn: async () => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listTelemetrySummary(instanceId ? { instanceId } : {});
    },
  });
  return {
    summary: q.data ?? null,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isLoading: q.isLoading,
    error: q.isError && toUiError(q.error).kind !== "not-wired" ? toUiError(q.error) : null,
    refetch: () => void q.refetch(),
  };
}
