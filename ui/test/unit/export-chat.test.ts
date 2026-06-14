/** PR-4.1 chat export — pure serialization + a safe filename. */

import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../../src/lib/chat-thread";
import { exportChatFilename, exportChatJson } from "../../src/lib/export-chat";

const user: ChatMessage = { id: "u1", role: "user", text: "hello", status: "done" };
const assistant: ChatMessage = {
  id: "a1",
  role: "assistant",
  text: "<think>hmm</think>hi there",
  status: "done",
  instanceId: "11".repeat(16),
  terminalMoteId: "22".repeat(32),
};

describe("exportChatJson", () => {
  it("serializes name + thread with run attribution, stable + parseable", () => {
    const json = exportChatJson("My chat", [user, assistant], { createdAt: 1, updatedAt: 2 });
    const parsed = JSON.parse(json);
    expect(parsed.kind).toBe("kortecx.chat");
    expect(parsed.version).toBe(1);
    expect(parsed.name).toBe("My chat");
    expect(parsed.created_at).toBe(1);
    expect(parsed.messages).toHaveLength(2);
    expect(parsed.messages[1]).toMatchObject({
      id: "a1",
      role: "assistant",
      text: "<think>hmm</think>hi there",
      instanceId: "11".repeat(16),
      terminalMoteId: "22".repeat(32),
    });
  });

  it("drops blob: previews — only the server-derived ref survives", () => {
    const withAttach: ChatMessage = {
      ...user,
      attachments: [
        { ref: "ab".repeat(32), filename: "x.png", mediaType: "image/png", objectUrl: "blob:xyz" },
      ],
    };
    const parsed = JSON.parse(exportChatJson("c", [withAttach]));
    const att = parsed.messages[0].attachments[0];
    expect(att).toEqual({ ref: "ab".repeat(32), filename: "x.png", mediaType: "image/png" });
    expect(att.objectUrl).toBeUndefined();
  });
});

describe("exportChatFilename", () => {
  it("slugs the name and is never empty or path-bearing", () => {
    expect(exportChatFilename("Plan a Trip!", 5)).toBe("kortecx-chat-plan-a-trip-5.json");
    expect(exportChatFilename("   ", 9)).toBe("kortecx-chat-chat-9.json");
    expect(exportChatFilename("a/b\\c", 1)).toBe("kortecx-chat-a-b-c-1.json");
  });
});
