import { describe, expect, it } from "vitest";
import type { UiError } from "../../src/kx/errors";
import {
  type ChatThread,
  EMPTY_THREAD,
  chatReducer,
  isTurnInFlight,
} from "../../src/lib/chat-thread";

const ERR: UiError = {
  code: "unavailable",
  kind: "retry",
  title: "Gateway unreachable",
  message: "boom",
  retryable: true,
};

function send(state: ChatThread, text = "hi"): ChatThread {
  return chatReducer(state, { type: "user_send", userId: "u1", assistantId: "a1", text });
}

describe("chatReducer", () => {
  it("user_send appends a done user message and a pending assistant", () => {
    const s = send(EMPTY_THREAD, "hello");
    expect(s.messages).toHaveLength(2);
    expect(s.messages[0]).toMatchObject({ id: "u1", role: "user", text: "hello", status: "done" });
    expect(s.messages[1]).toMatchObject({
      id: "a1",
      role: "assistant",
      text: "",
      status: "pending",
    });
  });

  it("turn_started → thinking and records the run handles", () => {
    const s = chatReducer(send(EMPTY_THREAD), {
      type: "turn_started",
      assistantId: "a1",
      instanceId: "ab".repeat(16),
      terminalMoteId: "ee".repeat(32),
    });
    expect(s.messages[1]).toMatchObject({
      status: "thinking",
      instanceId: "ab".repeat(16),
      terminalMoteId: "ee".repeat(32),
    });
  });

  it("turn_thinking promotes pending→thinking but never regresses done", () => {
    const started = send(EMPTY_THREAD);
    const thinking = chatReducer(started, { type: "turn_thinking", assistantId: "a1" });
    expect(thinking.messages[1]?.status).toBe("thinking");
    const done = chatReducer(thinking, { type: "turn_done", assistantId: "a1", text: "ok" });
    const noop = chatReducer(done, { type: "turn_thinking", assistantId: "a1" });
    expect(noop.messages[1]?.status).toBe("done");
  });

  it("turn_done sets the assistant text + done", () => {
    const s = chatReducer(send(EMPTY_THREAD), {
      type: "turn_done",
      assistantId: "a1",
      text: "the answer",
    });
    expect(s.messages[1]).toMatchObject({ status: "done", text: "the answer" });
  });

  it("turn_failed carries the error", () => {
    const s = chatReducer(send(EMPTY_THREAD), {
      type: "turn_failed",
      assistantId: "a1",
      error: ERR,
    });
    expect(s.messages[1]?.status).toBe("failed");
    expect(s.messages[1]?.error).toBe(ERR);
  });

  it("preserves order across multiple turns", () => {
    let s = chatReducer(EMPTY_THREAD, {
      type: "user_send",
      userId: "u1",
      assistantId: "a1",
      text: "q1",
    });
    s = chatReducer(s, { type: "turn_done", assistantId: "a1", text: "r1" });
    s = chatReducer(s, { type: "user_send", userId: "u2", assistantId: "a2", text: "q2" });
    s = chatReducer(s, { type: "turn_done", assistantId: "a2", text: "r2" });
    expect(s.messages.map((m) => m.id)).toEqual(["u1", "a1", "u2", "a2"]);
    expect(s.messages.map((m) => m.text)).toEqual(["q1", "r1", "q2", "r2"]);
  });

  it("reset clears the thread", () => {
    expect(chatReducer(send(EMPTY_THREAD), { type: "reset" })).toBe(EMPTY_THREAD);
  });
});

describe("isTurnInFlight", () => {
  it("true while an assistant turn is pending/thinking, false once done", () => {
    const pending = send(EMPTY_THREAD);
    expect(isTurnInFlight(pending)).toBe(true);
    const done = chatReducer(pending, { type: "turn_done", assistantId: "a1", text: "x" });
    expect(isTurnInFlight(done)).toBe(false);
    const failed = chatReducer(pending, { type: "turn_failed", assistantId: "a1", error: ERR });
    expect(isTurnInFlight(failed)).toBe(false);
  });
  it("false for an empty thread", () => {
    expect(isTurnInFlight(EMPTY_THREAD)).toBe(false);
  });
});
