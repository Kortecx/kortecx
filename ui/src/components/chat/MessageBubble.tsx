import type { ReactNode } from "react";
import { useUploadPreview } from "../../kx/use-upload-preview";
import type { ChatMessage, MessageAttachment } from "../../lib/chat-thread";
import { DigestChip } from "../DigestChip";
import { ErrorNotice } from "../ErrorNotice";
import { renderMarkdown } from "./markdown";

/** One attachment figure. A LIVE thread previews from the session-local `blob:`
 *  URL of the user's own file; a RESTORED thread re-resolves the bytes through
 *  the uploads scope (same server-derived ref). No URL resolves ⇒ chip only. */
function AttachmentFigure({ attachment }: { attachment: MessageAttachment }) {
  const restored = useUploadPreview(
    attachment.ref,
    attachment.mediaType,
    attachment.objectUrl === undefined,
  );
  const src = attachment.objectUrl ?? restored;
  return (
    <figure className="bubble__attachment">
      {src !== null && attachment.mediaType.startsWith("image/") ? (
        <img src={src} alt={attachment.filename} />
      ) : null}
      <figcaption>
        <span title={attachment.filename}>{attachment.filename}</span>
        <DigestChip hex={attachment.ref} label={attachment.filename} />
      </figcaption>
    </figure>
  );
}

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
    <>
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
              <AttachmentFigure key={a.ref} attachment={a} />
            ))}
          </div>
        ) : null}
        {/* Assistant replies render as Markdown (React elements only — never
            innerHTML); the user's own message stays verbatim. The container keeps
            the `bubble__text` class so existing rules + text-content assertions
            still match. */}
        {message.text ? (
          message.role === "assistant" ? (
            <div className="bubble__text bubble__md" data-testid="bubble-md">
              {renderMarkdown(message.text)}
            </div>
          ) : (
            <p className="bubble__text">{message.text}</p>
          )
        ) : null}
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
      </div>
      {/* T-FIX1: the DAG-of-thought trace renders as a SIBLING of the bubble so it
          spans the full chat column instead of being clamped to the 760px bubble. */}
      {trace}
    </>
  );
}
