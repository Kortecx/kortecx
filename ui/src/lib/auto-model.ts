import type { ModelSummary } from "@kortecx/sdk/web";

/**
 * Resolve the model "Auto" defers to — the Model Control v2 order, SHARED by every
 * surface so the picker's "Auto · X" LABEL can never diverge from what the runtime
 * actually binds: the server's ACTIVE model, then this browser's client-local default
 * (only if it is still served — never name a stale/unserved model), then the first
 * listed. Returns undefined only when nothing is served.
 *
 * Both the composer `ModelPicker` (the label) and `useChatController` (the modelId it
 * sends) call this, so a client-local default can no longer silently override the
 * server-active model the label promises.
 */
export function resolveAutoModel(
  models: readonly ModelSummary[] | undefined,
  defaultModelId: string | undefined,
): string | undefined {
  if (!models || models.length === 0) {
    return undefined;
  }
  const active = models.find((m) => m.active)?.modelId;
  if (active) {
    return active;
  }
  if (defaultModelId && models.some((m) => m.modelId === defaultModelId)) {
    return defaultModelId;
  }
  return models[0]?.modelId;
}
