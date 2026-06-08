import type { ReactNode } from "react";
import type { ChatMessage } from "../../lib/chat-thread";
import { ErrorNotice } from "../ErrorNotice";

/** One chat bubble (user/assistant). `trace` slots the assistant's DAG-of-thought. */
export function MessageBubble({ message, trace }: { message: ChatMessage; trace?: ReactNode }) {
  const inFlight = message.status === "pending" || message.status === "thinking";
  const mod = message.status === "failed" ? " bubble--failed" : inFlight ? " bubble--thinking" : "";
  return (
    <div
      className={`bubble bubble--${message.role}${mod}`}
      data-testid={`bubble-${message.role}`}
      data-status={message.status}
    >
      {message.role === "assistant" && inFlight ? (
        <span className="bubble__pending" data-testid="bubble-thinking">
          thinking…
        </span>
      ) : null}
      {message.text ? <p className="bubble__text">{message.text}</p> : null}
      {message.error ? <ErrorNotice error={message.error} /> : null}
      {trace}
    </div>
  );
}
