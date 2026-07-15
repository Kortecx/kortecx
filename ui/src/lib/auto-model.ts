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

/** The model the runtime will actually bind, plus what the picker must disclose. */
export interface BoundModel {
  /** The bound model — undefined ONLY when nothing is served. Carries `chatHandle`
   *  with it, so the id a turn sends and the recipe it routes to cannot disagree. */
  readonly model: ModelSummary | undefined;
  /** True iff an explicit pick named a SERVED model and was honored; false ⇒ Auto. */
  readonly explicit: boolean;
  /** An explicit pick this serve does NOT serve, so Auto bound instead. The picker
   *  discloses it. undefined when the pick was honored, absent, or still loading. */
  readonly stalePick: string | undefined;
}

/**
 * Resolve the model a chat turn BINDS. The single source both the `ModelPicker`
 * (the label) and `useChatController` (the id it sends + the `chatHandle` it routes
 * to) derive from, so the two can never disagree.
 *
 * An explicit pick is honored ONLY if it names a currently-served model; otherwise
 * this falls through to [`resolveAutoModel`]. That reconciliation is the whole point:
 * a pick persists in localStorage under a GLOBAL key (`kortecx.ui.chat` — no endpoint,
 * unlike `chat-history`), so a pick made against one serve outlives it and reappears
 * against a serve that never had that model. Honoring it blindly sent a stale enum
 * (GR15) while the picker — which already falls back to "Auto · X" for an unserved
 * value — promised a different model. Plain chat routes by `chatHandle` alone, so the
 * turn silently ran on whatever `models[0]` happened to be.
 *
 * Reconciled at READ time; the persisted pick is never rewritten. A pick is INTENT —
 * offloading a model to free VRAM must not destroy it, and it returns intact the
 * moment the model is served again. The sibling `handle` field is reconciled the same
 * way (`resolveChatBacking`), against the live recipe list, also without writing back.
 */
export function resolveBoundModel(
  models: readonly ModelSummary[] | undefined,
  pickedModelId: string | undefined,
  defaultModelId: string | undefined,
): BoundModel {
  const picked = pickedModelId ? models?.find((m) => m.modelId === pickedModelId) : undefined;
  if (picked) {
    return { model: picked, explicit: true, stalePick: undefined };
  }
  const auto = resolveAutoModel(models, defaultModelId);
  return {
    model: auto ? models?.find((m) => m.modelId === auto) : undefined,
    explicit: false,
    // Only once a non-empty list has LANDED — `useModels` reports `undefined` while
    // loading and on reconnect, and a pick is not stale just because nothing arrived.
    stalePick: pickedModelId && models && models.length > 0 ? pickedModelId : undefined,
  };
}
