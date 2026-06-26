/** Batch A chat-thread additions: attachments on user messages, the retry
 *  re-arm, and the form-gated vision arg planner. Pure — no network, no DOM. */

import { describe, expect, it } from "vitest";
import type { UiError } from "../../src/kx/errors";
import { planReactArgs, planReactVisionArgs, planVisionArgs } from "../../src/kx/use-chat";
import { type ChatThread, EMPTY_THREAD, chatReducer, retrySource } from "../../src/lib/chat-thread";

const ERR: UiError = {
  code: "unavailable",
  kind: "retry",
  title: "Gateway unreachable",
  message: "boom",
  retryable: true,
};

const ATT = { ref: "cd".repeat(32), filename: "cat.png", mediaType: "image/png" };

function sendWithAttachment(state: ChatThread): ChatThread {
  return chatReducer(state, {
    type: "user_send",
    userId: "u1",
    assistantId: "a1",
    text: "what is this?",
    attachments: [ATT],
  });
}

describe("chatReducer attachments + retry", () => {
  it("user_send carries attachments on the user message and pairs the assistant", () => {
    const s = sendWithAttachment(EMPTY_THREAD);
    expect(s.messages[0]?.attachments).toEqual([ATT]);
    expect(s.messages[1]?.forUserId).toBe("u1");
  });

  it("turn_retry re-arms ONLY a failed turn", () => {
    const failed = chatReducer(sendWithAttachment(EMPTY_THREAD), {
      type: "turn_failed",
      assistantId: "a1",
      error: ERR,
    });
    const retried = chatReducer(failed, { type: "turn_retry", assistantId: "a1" });
    expect(retried.messages[1]).toMatchObject({ status: "pending", text: "" });
    expect(retried.messages[1]?.error).toBeUndefined();

    // A done turn must NOT re-arm.
    const done = chatReducer(sendWithAttachment(EMPTY_THREAD), {
      type: "turn_done",
      assistantId: "a1",
      text: "a cat",
    });
    const noop = chatReducer(done, { type: "turn_retry", assistantId: "a1" });
    expect(noop.messages[1]?.status).toBe("done");
  });

  it("retrySource yields the paired user text + attachments for a FAILED turn only", () => {
    const failed = chatReducer(sendWithAttachment(EMPTY_THREAD), {
      type: "turn_failed",
      assistantId: "a1",
      error: ERR,
    });
    expect(retrySource(failed, "a1")).toEqual({
      text: "what is this?",
      attachments: [ATT],
      context: [],
    });
    // Not failed ⇒ no source.
    expect(retrySource(sendWithAttachment(EMPTY_THREAD), "a1")).toBeNull();
    expect(retrySource(failed, "nope")).toBeNull();
  });
});

type Field = {
  name: string;
  type: "str" | "enum" | "bytes";
  required: boolean;
  maxLen: number | null;
  allowed: readonly string[];
};
const field = (name: string, allowed: string[] = []): Field => ({
  name,
  type: allowed.length > 0 ? "enum" : "str",
  required: true,
  maxLen: null,
  allowed,
});

describe("planVisionArgs (form-gated — never send an undeclared arg)", () => {
  it("returns null when the form has no image_ref slot", () => {
    expect(planVisionArgs({ fields: [field("prompt")] }, "hi", ATT.ref, undefined)).toBeNull();
  });

  it("binds prompt + image_ref when declared", () => {
    const args = planVisionArgs(
      { fields: [field("prompt"), field("image_ref")] },
      "hi",
      ATT.ref,
      undefined,
    );
    expect(args).toEqual({ prompt: "hi", image_ref: ATT.ref });
  });

  it("rides the picked model when allowed, else the form's first legal value", () => {
    const fields = [field("prompt"), field("image_ref"), field("model", ["kx-serve:vlm"])];
    expect(planVisionArgs({ fields }, "hi", ATT.ref, "kx-serve:vlm")).toMatchObject({
      model: "kx-serve:vlm",
    });
    // A picked model OUTSIDE the allowed set falls back to a legal value —
    // the server stays the validator; the client just avoids a refused round trip.
    expect(planVisionArgs({ fields }, "hi", ATT.ref, "not-served")).toMatchObject({
      model: "kx-serve:vlm",
    });
  });
});

describe("chatReducer load_thread (chat history restore)", () => {
  it("replaces the whole message list", () => {
    const live = sendWithAttachment(EMPTY_THREAD);
    const restored = chatReducer(live, {
      type: "load_thread",
      messages: [
        { id: "x1", role: "user", text: "from history", status: "done" },
        { id: "x2", role: "assistant", text: "the saved reply", status: "done", forUserId: "x1" },
      ],
    });
    expect(restored.messages.map((m) => m.id)).toEqual(["x1", "x2"]);
    expect(restored.messages[1]?.text).toBe("the saved reply");
  });
});

// --- PR-2.1 agent mode: the form-gated react arg plan -------------------------

describe("planReactArgs", () => {
  const field = (name: string) => ({
    name,
    type: "str" as const,
    required: true,
    maxLen: null,
    allowed: [] as string[],
  });

  it("binds the instruction + declared budget caps (8/6 defaults)", () => {
    const form = { fields: [field("instruction"), field("max_turns"), field("max_tool_calls")] };
    expect(planReactArgs(form, "summarize the incident")).toEqual({
      instruction: "summarize the incident",
      max_turns: 8,
      max_tool_calls: 6,
    });
  });

  it("sends ONLY declared slots (never an undeclared arg — fail-closed binding)", () => {
    const form = { fields: [field("instruction")] };
    expect(planReactArgs(form, "task")).toEqual({ instruction: "task" });
  });

  it("a form without the instruction slot yields null (fall back to chat)", () => {
    expect(planReactArgs({ fields: [field("prompt")] }, "task")).toBeNull();
  });
});

describe("planReactVisionArgs (AGENTIC-VISION — agent mode + image)", () => {
  const field = (name: string) => ({
    name,
    type: "str" as const,
    required: true,
    maxLen: null,
    allowed: [] as string[],
  });

  it("binds the react args PLUS image_ref when the form declares both", () => {
    const form = {
      fields: [
        field("instruction"),
        field("max_turns"),
        field("max_tool_calls"),
        field("image_ref"),
      ],
    };
    expect(planReactVisionArgs(form, "inspect the chart", ATT.ref)).toEqual({
      instruction: "inspect the chart",
      max_turns: 8,
      max_tool_calls: 6,
      image_ref: ATT.ref,
    });
  });

  it("yields null without the image_ref slot (agent mode honest-degrades to text-only)", () => {
    const form = { fields: [field("instruction"), field("max_turns"), field("max_tool_calls")] };
    expect(planReactVisionArgs(form, "task", ATT.ref)).toBeNull();
  });

  it("yields null without the instruction slot (fall back)", () => {
    expect(planReactVisionArgs({ fields: [field("image_ref")] }, "task", ATT.ref)).toBeNull();
  });
});
