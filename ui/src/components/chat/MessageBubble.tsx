import type { ReactNode } from "react";
import type { ChatMessage } from "../../lib/chat-thread";
import { DigestChip } from "../DigestChip";
import { ErrorNotice } from "../ErrorNotice";

/** One chat bubble (user/assistant). `trace` slots the assistant's DAG-of-thought;
 *  `onRetry` arms the failed-turn retry (identical args — the server dedups). */
export function MessageBubble({
  message,
  trace,
  onRetry,
}: {
  message: ChatMessage;
  trace?: ReactNode;
  onRetry?: (assistantId: string) => void;
}) {
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
      {message.attachments && message.attachments.length > 0 ? (
        <div className="bubble__attachments" data-testid="bubble-attachments">
          {message.attachments.map((a) => (
            <figure key={a.ref} className="bubble__attachment">
              {a.objectUrl && a.mediaType.startsWith("image/") ? (
                // The session-local blob: preview of the user's OWN file —
                // untrusted server bytes never render as media here.
                <img src={a.objectUrl} alt={a.filename} />
              ) : null}
              <figcaption>
                <span title={a.filename}>{a.filename}</span>
                <DigestChip hex={a.ref} label={a.filename} />
              </figcaption>
            </figure>
          ))}
        </div>
      ) : null}
      {message.text ? <p className="bubble__text">{message.text}</p> : null}
      {message.error ? <ErrorNotice error={message.error} /> : null}
      {message.status === "failed" && onRetry ? (
        <button
          type="button"
          className="linkbtn bubble__retry"
          onClick={() => onRetry(message.id)}
          data-testid="retry-turn"
        >
          Retry (identical args — the runtime dedups)
        </button>
      ) : null}
      {trace}
    </div>
  );
}
