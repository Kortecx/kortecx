/**
 * Model Control v2 — the server-side ACTIVE default model (`SetActiveModel` +
 * `GetServerInfo.active_model_id`). An off-journal advisory hint (SN-8): the server
 * never re-routes `kx/recipes/chat`; this is just the default a client resolves to.
 * Switchable from any surface (a client-local default cannot be read by another).
 * On success it invalidates the models + server-info queries so `active` re-reads live.
 */

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useActiveModel() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();

  const setActive = useMutation<string, unknown, string>({
    mutationFn: async (modelId: string) => {
      if (!client) {
        throw new Error("not connected");
      }
      // An empty id CLEARS the override (back to the primary).
      return client.setActiveModel(modelId);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: queryKeys.models(endpoint) });
      void qc.invalidateQueries({ queryKey: queryKeys.serverInfo(endpoint) });
    },
  });

  return { setActive };
}
