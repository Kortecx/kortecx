import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type ChatSettings,
  DEFAULT_CHAT_SETTINGS,
  ECHO_PRESET,
  MODEL_CHAT_HANDLE,
  loadChatSettings,
  resolveChatBacking,
  saveChatSettings,
} from "../../src/lib/chat-settings";

const echoSettings: ChatSettings = {
  handle: ECHO_PRESET.handle,
  promptKey: ECHO_PRESET.promptKey,
  showThinking: true,
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
});
