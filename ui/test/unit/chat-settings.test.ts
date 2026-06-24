import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type ChatSettings,
  DEFAULT_CHAT_SETTINGS,
  ECHO_PRESET,
  MODEL_CHAT_HANDLE,
  loadChatSettings,
  resolveChatBacking,
  saveChatSettings,
  shouldPromptNoModel,
} from "../../src/lib/chat-settings";

const echoSettings: ChatSettings = {
  handle: ECHO_PRESET.handle,
  promptKey: ECHO_PRESET.promptKey,
  showThinking: true,
  showReasoning: true,
  autoscroll: true,
};

afterEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe("chat settings persistence", () => {
  it("returns defaults when nothing is stored", () => {
    expect(loadChatSettings()).toEqual(DEFAULT_CHAT_SETTINGS);
  });

  it("save then load round-trips", () => {
    const next = {
      handle: "kx/recipes/echo",
      promptKey: "topic",
      showThinking: false,
      showReasoning: false,
      autoscroll: false,
    };
    saveChatSettings(next);
    expect(loadChatSettings()).toEqual(next);
  });

  it("merges a partial stored value over defaults", () => {
    localStorage.setItem(
      "kortecx.ui.chat",
      JSON.stringify({ handle: "kx/recipes/passthrough-dag" }),
    );
    const s = loadChatSettings();
    expect(s.handle).toBe("kx/recipes/passthrough-dag");
    expect(s.promptKey).toBe(DEFAULT_CHAT_SETTINGS.promptKey);
    expect(s.showThinking).toBe(DEFAULT_CHAT_SETTINGS.showThinking);
  });

  it("ignores a blank handle / wrong types", () => {
    localStorage.setItem(
      "kortecx.ui.chat",
      JSON.stringify({ handle: "   ", promptKey: 5, showThinking: "yes" }),
    );
    expect(loadChatSettings()).toEqual(DEFAULT_CHAT_SETTINGS);
  });

  it("corrupt JSON → defaults (no throw)", () => {
    localStorage.setItem("kortecx.ui.chat", "{not json");
    expect(loadChatSettings()).toEqual(DEFAULT_CHAT_SETTINGS);
  });

  it("storage-unavailable on read → defaults; on write → no throw", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(loadChatSettings()).toEqual(DEFAULT_CHAT_SETTINGS);
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(() => saveChatSettings(DEFAULT_CHAT_SETTINGS)).not.toThrow();
  });
});

describe("resolveChatBacking — the live-recipe reconciliation (GR15)", () => {
  const withModel = [MODEL_CHAT_HANDLE, ECHO_PRESET.handle, "kx/recipes/react"];
  const modelFree = [ECHO_PRESET.handle];

  it("a STALE echo handle does NOT echo when the model chat recipe is served", () => {
    // The core bug: a globally-persisted model-free `echo` handle must not silently
    // echo the prompt — the model chat recipe backs chat whenever provisioned.
    expect(resolveChatBacking(echoSettings, withModel)).toEqual({
      handle: MODEL_CHAT_HANDLE,
      promptKey: "prompt",
    });
  });

  it("the default chat handle stays the model chat recipe", () => {
    expect(resolveChatBacking(DEFAULT_CHAT_SETTINGS, withModel)).toEqual({
      handle: MODEL_CHAT_HANDLE,
      promptKey: "prompt",
    });
  });

  it("echo backs chat on a MODEL-FREE serve (the honest degraded fallback)", () => {
    expect(resolveChatBacking(echoSettings, modelFree)).toEqual({
      handle: ECHO_PRESET.handle,
      promptKey: ECHO_PRESET.promptKey,
    });
  });

  it("honors a deliberate, available NON-echo handle", () => {
    const custom: ChatSettings = { ...echoSettings, handle: "kx/recipes/react", promptKey: "x" };
    expect(resolveChatBacking(custom, withModel)).toEqual({
      handle: "kx/recipes/react",
      promptKey: "x",
    });
  });

  it("an unavailable NON-echo handle is honored verbatim (the invoke fails honestly — don't-fake-gaps)", () => {
    const gone: ChatSettings = {
      ...echoSettings,
      handle: "kx/recipes/does-not-exist",
      promptKey: "p",
    };
    expect(resolveChatBacking(gone, withModel)).toEqual({
      handle: "kx/recipes/does-not-exist",
      promptKey: "p",
    });
  });

  it("while recipes are still loading (empty list) the persisted handle stands", () => {
    expect(resolveChatBacking(echoSettings, [])).toEqual({
      handle: ECHO_PRESET.handle,
      promptKey: ECHO_PRESET.promptKey,
    });
  });

  // POC-3: per-model routing — a chosen model routes the turn to its OWN recipe.
  it("routes the model chat turn to the chosen model's per-model handle when provisioned", () => {
    const avail = [MODEL_CHAT_HANDLE, "kx/recipes/m-gemma", ECHO_PRESET.handle];
    expect(resolveChatBacking(DEFAULT_CHAT_SETTINGS, avail, "kx/recipes/m-gemma")).toEqual({
      handle: "kx/recipes/m-gemma",
      promptKey: "prompt",
    });
  });

  it("the PRIMARY model's handle (kx/recipes/chat) is a no-op route (byte-identical)", () => {
    expect(resolveChatBacking(DEFAULT_CHAT_SETTINGS, withModel, MODEL_CHAT_HANDLE)).toEqual({
      handle: MODEL_CHAT_HANDLE,
      promptKey: "prompt",
    });
  });

  it("never routes a deliberate echo/custom choice to a model handle", () => {
    // On echo (a model-free choice) a per-model handle must NOT hijack the turn.
    expect(resolveChatBacking(echoSettings, [ECHO_PRESET.handle], "kx/recipes/m-gemma")).toEqual({
      handle: ECHO_PRESET.handle,
      promptKey: ECHO_PRESET.promptKey,
    });
  });

  it("a chosen model whose recipe is NOT provisioned falls back to the base handle", () => {
    // The per-model recipe isn't in the live list ⇒ no fake route (don't-fake-gaps).
    expect(resolveChatBacking(DEFAULT_CHAT_SETTINGS, withModel, "kx/recipes/m-absent")).toEqual({
      handle: MODEL_CHAT_HANDLE,
      promptKey: "prompt",
    });
  });
});

describe("shouldPromptNoModel — the proactive no-model honest-empty (GR15 §2.208)", () => {
  const base = {
    modelCount: 0,
    loading: false,
    unsupported: false,
    backingHandle: MODEL_CHAT_HANDLE,
  };

  it("prompts on a no-model serve with the default (non-echo) backing", () => {
    expect(shouldPromptNoModel(base)).toBe(true);
  });

  it("does NOT prompt when echo is the deliberate backing (honored verbatim)", () => {
    expect(shouldPromptNoModel({ ...base, backingHandle: ECHO_PRESET.handle })).toBe(false);
  });

  it("does NOT prompt while ListModels is still loading", () => {
    expect(shouldPromptNoModel({ ...base, modelCount: undefined, loading: true })).toBe(false);
  });

  it("does NOT prompt when a model IS provisioned", () => {
    expect(shouldPromptNoModel({ ...base, modelCount: 2 })).toBe(false);
  });

  it("does NOT prompt on a gateway that predates ListModels (unsupported)", () => {
    expect(shouldPromptNoModel({ ...base, modelCount: undefined, unsupported: true })).toBe(false);
  });
});
