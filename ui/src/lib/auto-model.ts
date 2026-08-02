import type { ModelSummary } from "@kortecx/sdk/web";

/**
 * Can this model answer a CHAT turn?
 *
 * An embedding model exposes only an embed endpoint; asking it to chat fails at dispatch.
 * The gateway already says which is which — `serving` marks the primary chat route and
 * `canEmbed` marks the embedder — so a model that only embeds is never a chat candidate.
 * Kept deliberately permissive otherwise: an entry that claims neither flag is still
 * offered, because a serve may list a chat model it has not marked primary.
 */
function isChatCapable(m: ModelSummary): boolean {
  return m.serving === true || m.canEmbed !== true;
}

/**
 * Resolve the model "Auto" defers to — the Model Control v2 order, SHARED by every
 * surface so the picker's "Auto · X" LABEL can never diverge from what the runtime
 * actually binds: the server's ACTIVE model, then this browser's client-local default
 * (only if it is still served — never name a stale/unserved model), then the model the
 * server is SERVING, then the first chat-capable entry. Returns undefined only when
 * nothing chat-capable is served.
 *
 * Both the composer `ModelPicker` (the label) and `useChatController` (the modelId it
 * sends) call this, so a client-local default can no longer silently override the
 * server-active model the label promises.
 *
 * **Why `serving` comes before "first listed".** The last two steps used to be a bare
 * `models[0]`, which is whatever sorts first in the catalog — and on a fresh install
 * nothing is server-active and the browser has no local default, so that fallback IS the
 * common path. A serve with an embedder registered lists it first, so New Chat opened on
 * a model whose only endpoint is `/api/embed` and the first message dead-lettered. The
 * gateway published `serving` all along; this just reads it. A substitute that cannot
 * satisfy the request is not a fallback, it is a deferred failure.
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
  const serving = models.find((m) => m.serving === true)?.modelId;
  if (serving) {
    return serving;
  }
  // Nothing is marked primary: take the first entry that could answer a chat turn, and
  // fall back to the first entry only when none qualifies — an embed-only serve then
  // still names something rather than rendering a picker with no answer at all.
  return (models.find(isChatCapable) ?? models[0])?.modelId;
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
 * while the picker — which already falls back to "Auto · X" for an unserved
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
