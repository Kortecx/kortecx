/**
 * POC-3 model lifecycle — the `LoadModel` / `OffloadModel` mutations that warm /
 * evict a REGISTERED local model in the gateway's owner-thread LRU. On success
 * they invalidate the models query so `loaded` residency re-reads live. An
 * unregistered id is a fail-closed `not found`; an FFI-free / old gateway
 * degrades (the controls surface the error honestly, never a fake success).
 *
 * load/offload only manage RAM residency — never authority. Selection /
 * routing stays the recipe `chatHandle` the server validates.
 */

import type { ModelLifecycleResult } from "@kortecx/sdk/web";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useModelLifecycle() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: queryKeys.models(endpoint) });
  };

  const load = useMutation<ModelLifecycleResult, unknown, string>({
    mutationFn: async (modelId: string) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.loadModel(modelId);
    },
    onSuccess: invalidate,
  });

  /**
   * Offload. The IN-USE GUARD means a "successful" call can have done NOTHING: when live
   * work holds the model the server refuses and returns `refused = true` with the
   * holders, rather than erroring. Callers must read the RESULT, not just the absence of
   * an error — treating this mutation's success as "the model is gone" is exactly the
   * misreading the guard exists to prevent.
   *
   * Pass `force` to override, which disrupts the listed holders.
   */
  const offload = useMutation<ModelLifecycleResult, unknown, { modelId: string; force?: boolean }>({
    mutationFn: async ({ modelId, force }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.offloadModel(modelId, { force });
    },
    // Invalidate only when residency ACTUALLY changed. A refusal changed nothing, and
    // re-reading the models query would repaint an unchanged row as if something had.
    onSuccess: (result) => {
      if (!result.refused) {
        invalidate();
      }
    },
  });

  return { load, offload };
}
