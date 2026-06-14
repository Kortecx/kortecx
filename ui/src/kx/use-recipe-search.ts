/**
 * Advisory recipe discovery (`SearchRecipes`, PR-4 Batch D) — rank the gateway's
 * provisioned recipes against a free-text intent. SN-8/display-only: a hit
 * SURFACES a recipe, never invokes one (`Invoke` stays the gate). An old gateway
 * (or a catalog with no ranker) degrades to `unsupported` — the search box hides,
 * no error UI.
 */

import type { ScoredRecipe } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";

export interface UseRecipeSearch {
  readonly results: readonly ScoredRecipe[] | undefined;
  readonly unsupported: boolean;
  readonly loading: boolean;
}

/** Search recipes for `intent`. An empty/blank intent is idle (no RPC). */
export function useRecipeSearch(intent: string): UseRecipeSearch {
  const { client, endpoint, status } = useConnection();
  const trimmed = intent.trim();
  const query = useQuery({
    queryKey: ["recipe-search", endpoint, trimmed],
    enabled: status === "connected" && client !== null && trimmed.length > 0,
    staleTime: 60_000,
    retry: false,
    queryFn: async (): Promise<ScoredRecipe[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.searchRecipes(trimmed, { limit: 25 });
    },
  });
  return {
    results: query.data,
    unsupported: query.isError && toUiError(query.error).kind === "not-wired",
    loading: query.isLoading && query.isFetching,
  };
}
