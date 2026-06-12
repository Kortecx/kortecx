/** PR-1.1 chat history — pure localStorage store (per-endpoint, bounded,
 *  fail-closed; objectUrls stripped; in-flight turns downgraded on save). */

import { beforeEach, describe, expect, it } from "vitest";
import { chatTitle, deleteChat, loadChats, saveChat } from "../../src/lib/chat-history";
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
    saveChat(EP, "a", [msg({ text: "first chat" })], 100);
    saveChat(EP, "b", [msg({ text: "second chat" })], 200);
    saveChat(EP, "a", [msg({ text: "first chat" }), msg({ id: "u2", text: "more" })], 300);
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
    saveChat(EP, "a", messages, 100);
    const [chat] = loadChats(EP);
    expect(chat?.messages[0]?.attachments?.[0]?.objectUrl).toBeUndefined();
    expect(chat?.messages[0]?.attachments?.[0]?.ref).toBe("cd".repeat(32));
    // The interrupted turn restores re-armable, never a forever-spinner.
    expect(chat?.messages[1]?.status).toBe("failed");
    expect(chat?.messages[1]?.error?.retryable).toBe(true);
  });

  it("caps the history at 30 (newest kept)", () => {
    for (let i = 0; i < 35; i++) {
      saveChat(EP, `c${i}`, [msg({ text: `chat ${i}` })], i);
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
    saveChat(EP, "a", [msg()], 1);
    saveChat(EP, "b", [msg({ text: "other" })], 2);
    const left = deleteChat(EP, "a");
    expect(left.map((c) => c.id)).toEqual(["b"]);
    expect(loadChats(EP).map((c) => c.id)).toEqual(["b"]);
  });
});

describe("chatTitle", () => {
  it("is the first user message, whitespace-collapsed and truncated", () => {
    expect(chatTitle([msg({ text: "  what\n is \tthis? " })])).toBe("what is this?");
    expect(chatTitle([msg({ text: "x".repeat(100) })])).toBe(`${"x".repeat(64)}…`);
    expect(chatTitle([])).toBe("(empty chat)");
  });
});
