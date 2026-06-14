import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MessageBubble } from "../../src/components/chat/MessageBubble";
import type { ChatMessage } from "../../src/lib/chat-thread";

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
});
