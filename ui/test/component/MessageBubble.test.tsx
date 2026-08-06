import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "../../src/lib/chat-thread";

// The assistant action row (PR-4.1) embeds FeedbackButtons → useFeedback →
// useConnection; stub the mutation so the bubble renders without a provider.
vi.mock("../../src/kx/use-feedback", () => ({
  useFeedback: () => ({ mutate: vi.fn(), isError: false, error: null }),
}));
// The action row also links to the run behind the answer. `Link` needs a router; the
// established pattern in this suite is to stub the module (see run-scope.test.tsx) and
// serialize `search` onto the anchor so the SCOPE is assertable, not just the target.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, search, children, ...rest }: Record<string, unknown> & { children?: unknown }) => (
    <a
      href={typeof to === "string" ? to : "#"}
      data-search={JSON.stringify(search ?? {})}
      {...rest}
    >
      {children as never}
    </a>
  ),
}));

import { MessageBubble } from "../../src/components/chat/MessageBubble";

const msg = (m: Partial<ChatMessage> & Pick<ChatMessage, "role" | "status">): ChatMessage => ({
  id: "1",
  text: "",
  ...m,
});

describe("MessageBubble", () => {
  it("renders a user message", () => {
    render(<MessageBubble message={msg({ role: "user", status: "done", text: "hello" })} />);
    const bubble = screen.getByTestId("bubble-user");
    expect(bubble).toHaveTextContent("hello");
  });

  it("an in-flight assistant shows the thinking indicator", () => {
    render(<MessageBubble message={msg({ role: "assistant", status: "thinking" })} />);
    expect(screen.getByTestId("bubble-thinking")).toBeInTheDocument();
  });

  it("a failed assistant renders an error notice", () => {
    render(
      <MessageBubble
        message={msg({
          role: "assistant",
          status: "failed",
          error: {
            code: "run_failed",
            kind: "generic",
            title: "Run failed",
            message: "boom",
            retryable: false,
          },
        })}
      />,
    );
    expect(screen.getByTestId("error-notice")).toBeInTheDocument();
  });

  it("a done assistant renders its text and slots a trace", () => {
    render(
      <MessageBubble
        message={msg({ role: "assistant", status: "done", text: "the answer" })}
        trace={<div data-testid="trace-slot" />}
      />,
    );
    expect(screen.getByTestId("bubble-assistant")).toHaveTextContent("the answer");
    expect(screen.getByTestId("trace-slot")).toBeInTheDocument();
    // PR-4.1: a settled assistant answer carries the copy + 👍/👎 action row.
    expect(screen.getByTestId("msg-actions")).toBeInTheDocument();
    expect(screen.getByTestId("msg-copy")).toBeInTheDocument();
    expect(screen.getByTestId("msg-feedback-up")).toBeInTheDocument();
  });

  // T-FEAT1: split a leading <think> reasoning block into a disclosure.
  it("splits a leading <think> block into a Reasoning disclosure above the answer", () => {
    render(
      <MessageBubble
        message={msg({
          role: "assistant",
          status: "done",
          text: "<think>my reasoning</think>The answer.",
        })}
        showReasoning
      />,
    );
    const reasoning = screen.getByTestId("bubble-reasoning");
    expect(reasoning).toHaveTextContent("my reasoning");
    const answer = screen.getByTestId("bubble-md");
    expect(answer).toHaveTextContent("The answer.");
    expect(answer.textContent).not.toContain("my reasoning");
  });

  it("showReasoning=false hides the disclosure but keeps the answer", () => {
    render(
      <MessageBubble
        message={msg({ role: "assistant", status: "done", text: "<think>secret</think>Hello" })}
        showReasoning={false}
      />,
    );
    expect(screen.queryByTestId("bubble-reasoning")).toBeNull();
    expect(screen.getByTestId("bubble-md")).toHaveTextContent("Hello");
  });

  it("a reply with no <think> renders the answer with no disclosure", () => {
    render(
      <MessageBubble
        message={msg({ role: "assistant", status: "done", text: "Just an answer." })}
        showReasoning
      />,
    );
    expect(screen.queryByTestId("bubble-reasoning")).toBeNull();
    expect(screen.getByTestId("bubble-md")).toHaveTextContent("Just an answer.");
  });

  // PR-4.2 (T-STREAM1): the streaming render states.
  it("a thinking turn with streamed text marks the answer container as a live region", () => {
    render(<MessageBubble message={msg({ role: "assistant", status: "thinking", text: "Hel" })} />);
    const md = screen.getByTestId("bubble-md");
    expect(md).toHaveTextContent("Hel");
    expect(md).toHaveAttribute("data-streaming", "true");
    expect(md).toHaveAttribute("aria-live", "polite");
  });

  it("an agent chain's live reasoning streams into a secondary trace line, not the answer", () => {
    render(
      <MessageBubble
        message={msg({
          role: "assistant",
          status: "thinking",
          streamingReasoning: '{"tool":"echo","args":{}}',
        })}
      />,
    );
    const reasoning = screen.getByTestId("bubble-reasoning-stream");
    expect(reasoning).toHaveTextContent('{"tool":"echo","args":{}}');
    expect(reasoning).toHaveAttribute("aria-live", "polite");
    // The raw envelope is NOT rendered as the answer (no committed text yet).
    expect(screen.queryByTestId("bubble-md")).toBeNull();
  });

  it("a settled answer drops the live-region marker (reconciled to the committed fact)", () => {
    render(
      <MessageBubble message={msg({ role: "assistant", status: "done", text: "final answer" })} />,
    );
    const md = screen.getByTestId("bubble-md");
    expect(md).toHaveTextContent("final answer");
    expect(md).not.toHaveAttribute("data-streaming");
    expect(screen.queryByTestId("bubble-reasoning-stream")).toBeNull();
  });
});

/**
 * An answer IS a run. Chat held both anchor halves on every assistant message and
 * passed them only to RPCs, so the console's landing surface was the one place you
 * could not open the thing you were looking at. The mechanism was already shared —
 * `runViewSearch` — and twelve other surfaces used it.
 */
describe("MessageBubble — opening the run behind an answer", () => {
  const anchored = {
    role: "assistant" as const,
    status: "done" as const,
    text: "the answer",
    instanceId: "ab".repeat(16),
    terminalMoteId: "ee".repeat(32),
  };

  it("a settled answer with an anchor links to its own run, scoped", () => {
    render(<MessageBubble message={msg(anchored)} />);
    const link = screen.getByTestId("msg-open-run");
    expect(link).toBeInTheDocument();
    // Scoped, not just "the run view": without the anchor the view shows the whole
    // journal, and one `kx serve` shares an instance id across every submission.
    expect(link.getAttribute("data-search")).toContain("ee".repeat(32));
  });

  it("an answer with NO anchor offers no link rather than an unscoped one", () => {
    render(<MessageBubble message={msg({ ...anchored, terminalMoteId: undefined })} />);
    expect(screen.queryByTestId("msg-open-run")).not.toBeInTheDocument();
  });

  it("an unsettled answer offers no link (there is nothing settled to open)", () => {
    render(<MessageBubble message={msg({ ...anchored, status: "thinking" })} />);
    expect(screen.queryByTestId("msg-open-run")).not.toBeInTheDocument();
  });
});
