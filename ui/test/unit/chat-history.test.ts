/** PR-1.1 chat history — pure localStorage store (per-endpoint, bounded,
 *  fail-closed; objectUrls stripped; in-flight turns downgraded on save). */

import { beforeEach, describe, expect, it } from "vitest";
import {
  chatTitle,
  defaultChatName,
  deleteChat,
  loadChats,
  renameChat,
  saveChat,
} from "../../src/lib/chat-history";
import type { ChatMessage } from "../../src/lib/chat-thread";

const EP = "http://127.0.0.1:50151";

function msg(over: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: over.id ?? "u1",
    role: "user",
    text: "hello runtime",
    status: "done",
    ...over,
  };
}

beforeEach(() => {
  localStorage.clear();
});

describe("saveChat / loadChats", () => {
  it("upserts by id, newest-updated first, per endpoint", () => {
    saveChat(EP, "a", [msg({ text: "first chat" })], undefined, 100);
    saveChat(EP, "b", [msg({ text: "second chat" })], undefined, 200);
    saveChat(
      EP,
      "a",
      [msg({ text: "first chat" }), msg({ id: "u2", text: "more" })],
      undefined,
      300,
    );
    const chats = loadChats(EP);
    expect(chats.map((c) => c.id)).toEqual(["a", "b"]);
    expect(chats[0]?.messages).toHaveLength(2);
    expect(chats[0]?.createdAt).toBe(100); // upsert keeps the original createdAt
    expect(loadChats("http://other:1")).toEqual([]);
  });

  it("an empty thread is a no-op (nothing worth listing)", () => {
    saveChat(EP, "a", []);
    expect(loadChats(EP)).toEqual([]);
  });

  it("strips blob: objectUrls and downgrades in-flight turns on save", () => {
    const messages: ChatMessage[] = [
      msg({
        attachments: [
          { ref: "cd".repeat(32), filename: "x.png", mediaType: "image/png", objectUrl: "blob:x" },
        ],
      }),
      msg({ id: "a1", role: "assistant", text: "", status: "thinking", forUserId: "u1" }),
    ];
    saveChat(EP, "a", messages, undefined, 100);
    const [chat] = loadChats(EP);
    expect(chat?.messages[0]?.attachments?.[0]?.objectUrl).toBeUndefined();
    expect(chat?.messages[0]?.attachments?.[0]?.ref).toBe("cd".repeat(32));
    // The interrupted turn restores re-armable, never a forever-spinner.
    expect(chat?.messages[1]?.status).toBe("failed");
    expect(chat?.messages[1]?.error?.retryable).toBe(true);
  });

  it("caps the history at 30 (newest kept)", () => {
    for (let i = 0; i < 35; i++) {
      saveChat(EP, `c${i}`, [msg({ text: `chat ${i}` })], undefined, i);
    }
    const chats = loadChats(EP);
    expect(chats).toHaveLength(30);
    expect(chats[0]?.id).toBe("c34");
  });

  it("is fail-closed on a corrupt store", () => {
    localStorage.setItem(`kortecx.ui.chats:${EP}`, "{not json");
    expect(loadChats(EP)).toEqual([]);
    localStorage.setItem(`kortecx.ui.chats:${EP}`, JSON.stringify({ nope: 1 }));
    expect(loadChats(EP)).toEqual([]);
  });
});

describe("deleteChat", () => {
  it("forgets one chat and keeps the rest", () => {
    saveChat(EP, "a", [msg()], undefined, 1);
    saveChat(EP, "b", [msg({ text: "other" })], undefined, 2);
    const left = deleteChat(EP, "a");
    expect(left.map((c) => c.id)).toEqual(["b"]);
    expect(loadChats(EP).map((c) => c.id)).toEqual(["b"]);
  });
});

describe("chat name (editable, default timestamp, preserved across autosaves)", () => {
  it("persists the supplied name and PRESERVES it across a nameless autosave", () => {
    saveChat(EP, "a", [msg({ text: "hi" })], "My chat", 100);
    expect(loadChats(EP)[0]?.name).toBe("My chat");
    // A later autosave WITHOUT a name keeps the prior name (never recomputed).
    saveChat(EP, "a", [msg({ text: "hi" }), msg({ id: "u2", text: "more" })], undefined, 200);
    expect(loadChats(EP)[0]?.name).toBe("My chat");
  });

  it("defaultChatName returns a non-empty string for a given instant", () => {
    expect(defaultChatName(0).length).toBeGreaterThan(0);
  });

  it("renameChat updates one chat; a blank name is a no-op", () => {
    saveChat(EP, "a", [msg()], "orig", 1);
    renameChat(EP, "a", "  Renamed  ");
    expect(loadChats(EP)[0]?.name).toBe("Renamed");
    renameChat(EP, "a", "   ");
    expect(loadChats(EP)[0]?.name).toBe("Renamed"); // blank ignored
  });
});

describe("chatTitle", () => {
  it("is the first user message, whitespace-collapsed and truncated", () => {
    expect(chatTitle([msg({ text: "  what\n is \tthis? " })])).toBe("what is this?");
    expect(chatTitle([msg({ text: "x".repeat(100) })])).toBe(`${"x".repeat(64)}…`);
    expect(chatTitle([])).toBe("(empty chat)");
  });
});
