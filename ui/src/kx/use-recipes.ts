/**
 * The invocable recipe catalog (`ListRecipes`) + a single recipe's free-param form
 * (`GetRecipeForm`). Both tolerate a gateway that has not wired the recipe catalog
 * (UNIMPLEMENTED → the query errors; the Recipes view falls back to the manual
 * handle+JSON form). A recipe's form is immutable for a given provisioned gateway,
 * so it caches indefinitely.
 */

import type { RecipeForm, RecipeInfo } from "@kortecx/sdk/web";
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

/**
 * The fingerprint→handle naming map (PR-2.1): durable `RunSummary` rows carry
 * only a `recipeFingerprint`, so the Workflows list joins it here to label runs
 * by their recipe handle. Tolerant: a gateway predating the field (or without
 * the catalog) yields an EMPTY map — rows degrade to hex ids, never an error.
 */
export function useRecipeNames() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.recipeNames(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<Record<string, string>> => {
      if (!client) {
        throw new Error("not connected");
      }
      try {
        const summaries = await client.listRecipeSummaries();
        const map: Record<string, string> = {};
        for (const s of summaries) {
          if (s.recipeFingerprint !== "") {
            map[s.recipeFingerprint] = s.handle;
          }
        }
        return map;
      } catch {
        return {}; // not wired / old gateway — unlabeled rows are honest
      }
    },
  });
}

/**
 * The handle→summary metadata map (PR-4.1b Blueprint cards): the SAME
 * `ListRecipes` summaries `useRecipeNames` reads, but keyed by handle and
 * keeping the advisory `description`/`tags`/`version` the cards render as a
 * subtitle + chips. Tolerant: an old gateway (or no catalog) yields an EMPTY
 * map — cards degrade to the handle headline, never an error.
 */
export function useRecipeSummaries() {
  const { client, endpoint, status } = useConnection();
  return useQuery({
    queryKey: queryKeys.recipeSummaries(endpoint),
    enabled: status === "connected" && client !== null,
    queryFn: async (): Promise<Record<string, RecipeInfo>> => {
      if (!client) {
        throw new Error("not connected");
      }
      try {
        const summaries = await client.listRecipeSummaries();
        const map: Record<string, RecipeInfo> = {};
        for (const s of summaries) {
          map[s.handle] = s;
        }
        return map;
      } catch {
        return {}; // not wired / old gateway — handle-only cards are honest
      }
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
