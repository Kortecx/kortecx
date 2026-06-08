import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_CHAT_SETTINGS,
  loadChatSettings,
  saveChatSettings,
} from "../../src/lib/chat-settings";

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
    localStorage.setItem("kortecx.ui.chat", JSON.stringify({ handle: "kx/recipes/exec-demo" }));
    const s = loadChatSettings();
    expect(s.handle).toBe("kx/recipes/exec-demo");
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
