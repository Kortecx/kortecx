/**
 * POC-3 model lifecycle — the `LoadModel` / `OffloadModel` mutations that warm /
 * evict a REGISTERED local model in the gateway's owner-thread LRU. On success
 * they invalidate the models query so `loaded` residency re-reads live. An
 * unregistered id is a fail-closed `not found`; an FFI-free / old gateway
 * degrades (the controls surface the error honestly, never a fake success).
 *
 * SN-8: load/offload only manage RAM residency — never authority. Selection /
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

  const offload = useMutation<ModelLifecycleResult, unknown, string>({
    mutationFn: async (modelId: string) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.offloadModel(modelId);
    },
    onSuccess: invalidate,
  });

  return { load, offload };
}
