/**
 * The CLIENT-LOCAL default-model preference (POC-5c / D168). Mirrors chat-settings'
 * localStorage try/catch pattern — non-secret, best-effort, corruption-safe.
 *
 * It is a UI convenience ONLY: the New Chat composer pre-selects it when the user
 * has not explicitly picked a model, but the model still only ever rides as a
 * server-validated recipe ENUM free-param (SN-8) — choosing a default grants nothing
 * and routes nothing on its own. Per-browser (not shared across clients); a
 * server-side runtime default is a later POC-GATE / Cloud concern.
 */

const KEY = "kortecx.ui.default-model";

/** Fired on the window when the default changes in THIS tab (the `storage` event
 *  only fires in OTHER tabs) — lets a mounted picker/badge re-read immediately. */
export const DEFAULT_MODEL_CHANGED_EVENT = "kortecx:default-model-changed";

/** The saved default model id, or `undefined` when none is set / storage is unavailable. */
export function loadDefaultModel(): string | undefined {
  try {
    const raw = localStorage.getItem(KEY);
    return raw !== null && raw.trim() !== "" ? raw : undefined;
  } catch {
    return undefined;
  }
}

/** Persist the default model id (best-effort; storage may be unavailable in private mode). */
export function saveDefaultModel(modelId: string): void {
  try {
    localStorage.setItem(KEY, modelId);
    window.dispatchEvent(new Event(DEFAULT_MODEL_CHANGED_EVENT));
  } catch {
    /* best-effort */
  }
}

/** Clear the default (back to "the gateway/first-listed model"). */
export function clearDefaultModel(): void {
  try {
    localStorage.removeItem(KEY);
    window.dispatchEvent(new Event(DEFAULT_MODEL_CHANGED_EVENT));
  } catch {
    /* best-effort */
  }
}
