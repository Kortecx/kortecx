/** PR-4.1 FeedbackButtons — fires SubmitFeedback with the answer's target keys,
 *  selects optimistically, and HIDES on a not-wired gateway (don't-fake-gaps). */

import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "../../src/lib/chat-thread";

const mutate = vi.fn();
const state: { isError: boolean; error: unknown } = { isError: false, error: null };

vi.mock("../../src/kx/use-feedback", () => ({
  useFeedback: () => ({ mutate, isError: state.isError, error: state.error }),
}));
// Pass the test error through unchanged so we control its `kind` directly.
vi.mock("../../src/kx/errors", () => ({ toUiError: (e: unknown) => e }));

import { FeedbackButtons } from "../../src/components/chat/FeedbackButtons";

const msg: ChatMessage = {
  id: "a1",
  role: "assistant",
  text: "hi",
  status: "done",
  instanceId: "11".repeat(16),
  terminalMoteId: "22".repeat(32),
};

afterEach(() => {
  mutate.mockReset();
  state.isError = false;
  state.error = null;
});

describe("FeedbackButtons", () => {
  it("renders 👍/👎 and submits the rating with the answer's target keys", () => {
    render(<FeedbackButtons message={msg} recipeHandle="kx/recipes/chat" modelId="qwen3" />);
    fireEvent.click(screen.getByTestId("msg-feedback-up"));
    expect(mutate).toHaveBeenCalledTimes(1);
    expect(mutate.mock.calls[0]?.[0]).toMatchObject({
      rating: "up",
      messageId: "a1",
      instanceId: "11".repeat(16),
      moteId: "22".repeat(32),
      recipeHandle: "kx/recipes/chat",
      modelId: "qwen3",
    });
    expect(screen.getByTestId("msg-feedback-up").getAttribute("aria-pressed")).toBe("true");
  });

  it("hides entirely on a not-wired gateway", () => {
    state.isError = true;
    state.error = { kind: "not-wired" };
    const { container } = render(<FeedbackButtons message={msg} />);
    expect(container.firstChild).toBeNull();
    expect(screen.queryByTestId("msg-feedback-up")).toBeNull();
  });
});
