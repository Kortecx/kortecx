/**
 * Chat settings persistence — non-secret only, mirroring the endpoint-persistence
 * pattern in connection-context (localStorage with a try/catch fallback). The
 * bearer token is NEVER touched here. `load` merges over defaults defensively so a
 * corrupt/partial stored value can never crash the chat panel.
 */

export interface ChatSettings {
  /** Recipe handle that backs chat (default the model recipe; echo for model-free). */
  readonly handle: string;
  /** Free-param key the user message binds to (chat→`prompt`, echo→`topic`). */
  readonly promptKey: string;
  /** Show the DAG-of-thought (the in-flight run's Motes) under assistant turns. */
  readonly showThinking: boolean;
  /** Show the model's `<think>` REASONING block (a collapsible disclosure above
   *  the answer). DISTINCT from `showThinking` (the DAG): this is the model's own
   *  textual reasoning, already committed in the result bytes (display-only). */
  readonly showReasoning: boolean;
  /** Auto-scroll the thread to the newest message. */
  readonly autoscroll: boolean;
  /** The picked model id (Batch A). Only ever sent as a recipe ENUM free-param
   *  the SERVER validates — never authority. `undefined` = the gateway default. */
  readonly modelId?: string;
}

export const DEFAULT_CHAT_SETTINGS: ChatSettings = {
  handle: "kx/recipes/chat",
  promptKey: "prompt",
  showThinking: true,
  showReasoning: true,
  autoscroll: true,
};

/** A handy preset: the model-free `echo` recipe (deterministic round-trip). */
export const ECHO_PRESET: Pick<ChatSettings, "handle" | "promptKey"> = {
  handle: "kx/recipes/echo",
  promptKey: "topic",
};

/** The model chat recipe — the backer whenever a serve provisions inference. */
export const MODEL_CHAT_HANDLE = "kx/recipes/chat";

/**
 * Reconcile the (globally-)persisted chat handle against THIS serve's LIVE recipe
 * list (GR15 + D142.3 don't-fake-gaps). The model chat recipe backs chat WHENEVER
 * it is provisioned: a stale model-free `echo` handle — persisted from an earlier
 * no-model session, and now an HONEST passthrough — must never silently echo the
 * user's prompt back instead of answering it. A deliberate, available NON-echo
 * handle (e.g. a custom recipe) is still honored; `echo` backs chat only on a
 * model-free serve, where the DegradeNotice explains the round-trip. While the
 * recipe list is still loading (`available` empty) the persisted handle stands.
 */
export function resolveChatBacking(
  settings: ChatSettings,
  available: readonly string[],
): { handle: string; promptKey: string } {
  // The SINGLE reconciliation: a stale model-free `echo` handle must not silently
  // echo the prompt when the serve provisions the model chat recipe — prefer the
  // model. EVERY other handle is honored VERBATIM (a deliberate non-echo choice,
  // or even an intentionally-invalid one), so the invoke surfaces real failures
  // honestly instead of masking them (D142.3 don't-fake-gaps).
  if (settings.handle === ECHO_PRESET.handle && available.includes(MODEL_CHAT_HANDLE)) {
    return { handle: MODEL_CHAT_HANDLE, promptKey: "prompt" };
  }
  return { handle: settings.handle, promptKey: settings.promptKey };
}

/**
 * Whether to PROACTIVELY surface the "no model — connect one" guidance (GR15
 * §2.208): true only on a no-model serve (`ListModels` resolved to an empty list,
 * not loading, the RPC supported) AND when the backing is NOT a deliberate `echo`
 * choice. A chosen `echo` is honored verbatim (resolveChatBacking's contract) — it
 * is an explicit model-free round-trip, not a gap to flag. Pure (no React) so the
 * decision is unit-tested without rendering the whole panel.
 */
export function shouldPromptNoModel(opts: {
  /** `ListModels` count (`undefined` while loading / before the first response). */
  readonly modelCount: number | undefined;
  readonly loading: boolean;
  /** The gateway predates `ListModels` (don't prompt — we can't know). */
  readonly unsupported: boolean;
  /** The reconciled chat backing handle (from {@link resolveChatBacking}). */
  readonly backingHandle: string;
}): boolean {
  return (
    opts.modelCount === 0 &&
    !opts.loading &&
    !opts.unsupported &&
    opts.backingHandle !== ECHO_PRESET.handle
  );
}

const KEY = "kortecx.ui.chat";

function isString(v: unknown): v is string {
  return typeof v === "string";
}
function isBool(v: unknown): v is boolean {
  return typeof v === "boolean";
}

/** Load settings, merging any stored values over the defaults (corruption-safe). */
export function loadChatSettings(): ChatSettings {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) {
      return DEFAULT_CHAT_SETTINGS;
    }
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== "object") {
      return DEFAULT_CHAT_SETTINGS;
    }
    const p = parsed as Record<string, unknown>;
    return {
      handle:
        isString(p.handle) && p.handle.trim() !== "" ? p.handle : DEFAULT_CHAT_SETTINGS.handle,
      promptKey:
        isString(p.promptKey) && p.promptKey.trim() !== ""
          ? p.promptKey
          : DEFAULT_CHAT_SETTINGS.promptKey,
      showThinking: isBool(p.showThinking) ? p.showThinking : DEFAULT_CHAT_SETTINGS.showThinking,
      showReasoning: isBool(p.showReasoning)
        ? p.showReasoning
        : DEFAULT_CHAT_SETTINGS.showReasoning,
      autoscroll: isBool(p.autoscroll) ? p.autoscroll : DEFAULT_CHAT_SETTINGS.autoscroll,
      modelId: isString(p.modelId) && p.modelId.trim() !== "" ? p.modelId : undefined,
    };
  } catch {
    return DEFAULT_CHAT_SETTINGS;
  }
}

/** Persist settings (best-effort; storage may be unavailable in private mode). */
export function saveChatSettings(s: ChatSettings): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* best-effort */
  }
}
