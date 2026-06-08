/**
 * Query keys are ENDPOINT-SCOPED so switching gateways never serves a stale
 * cross-endpoint cache. `atSeq` distinguishes a pinned time-travel snapshot from
 * the live ("live") view.
 */

export const queryKeys = {
  signatures: (endpoint: string) => ["kx", endpoint, "signatures"] as const,
  projection: (endpoint: string, instanceId: string, atSeq?: number) =>
    ["kx", endpoint, "projection", instanceId, atSeq ?? "live"] as const,
  /** A committed artifact blob, scoped by its owning run + content ref. */
  content: (endpoint: string, instanceId: string, ref: string) =>
    ["kx", endpoint, "content", instanceId, ref] as const,
  /** Durable run enumeration (`ListRuns`), scoped by the requested page size. */
  runs: (endpoint: string, limit: number) => ["kx", endpoint, "runs", limit] as const,
  /** The invocable recipe catalog (`ListRecipes`). */
  recipes: (endpoint: string) => ["kx", endpoint, "recipes"] as const,
  /** One recipe's free-param form (`GetRecipeForm`), scoped by handle. */
  recipeForm: (endpoint: string, handle: string) =>
    ["kx", endpoint, "recipe-form", handle] as const,
  /** Gateway liveness probe (endpoint-scoped). */
  health: (endpoint: string) => ["kx", endpoint, "health"] as const,
};
