/**
 * Model discovery (`ListModels`, Batch A) — display-only (SN-8: listing a model
 * never routes one; selection stays a recipe ENUM free-param the server
 * validates). An OLD gateway without the RPC degrades to `unsupported` (the
 * picker hides); an FFI-free gateway returns an honest empty list.
 */

import type { ModelSummary } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

export interface UseModels {
  readonly models: readonly ModelSummary[] | undefined;
  /** True when the gateway predates ListModels (hide the picker, no error UI). */
  readonly unsupported: boolean;
  readonly loading: boolean;
}

export function useModels(): UseModels {
  const { client, endpoint, status } = useConnection();
  const query = useQuery({
    queryKey: queryKeys.models(endpoint),
    enabled: status === "connected" && client !== null,
    staleTime: Number.POSITIVE_INFINITY, // fixed for a provisioned gateway (restart to change)
    retry: false,
    queryFn: async (): Promise<ModelSummary[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listModels();
    },
  });
  return {
    models: query.data,
    unsupported: query.isError && toUiError(query.error).kind === "not-wired",
    loading: query.isLoading,
  };
}
