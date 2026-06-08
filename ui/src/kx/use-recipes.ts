/**
 * The invocable recipe catalog (`ListRecipes`) + a single recipe's free-param form
 * (`GetRecipeForm`). Both tolerate a gateway that has not wired the recipe catalog
 * (UNIMPLEMENTED → the query errors; the Recipes view falls back to the manual
 * handle+JSON form). A recipe's form is immutable for a given provisioned gateway,
 * so it caches indefinitely.
 */

import type { RecipeForm } from "@kortecx/sdk/web";
import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

export function useRecipes() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.recipes(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<string[]> => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.listRecipes();
    },
  });
}

export function useRecipeForm(handle: string | undefined) {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.recipeForm(endpoint, handle ?? ""),
    enabled: status === "connected" && client !== null && Boolean(handle),
    staleTime: Number.POSITIVE_INFINITY, // a recipe's form is fixed for the gateway
    queryFn: async (): Promise<RecipeForm> => {
      if (!client || !handle) {
        throw new Error("not connected");
      }
      return client.getRecipeForm(handle);
    },
  });
}
