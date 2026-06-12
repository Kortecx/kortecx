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
  autoscroll: true,
};

/** A handy preset: the model-free `echo` recipe (deterministic round-trip). */
export const ECHO_PRESET: Pick<ChatSettings, "handle" | "promptKey"> = {
  handle: "kx/recipes/echo",
  promptKey: "topic",
};

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
