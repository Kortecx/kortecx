import type { ReactNode } from "react";
import { useUploadPreview } from "../../kx/use-upload-preview";
import type { ChatMessage, MessageAttachment } from "../../lib/chat-thread";
import { splitReasoning } from "../../lib/split-reasoning";
import { useCopyToClipboard } from "../../lib/use-copy-to-clipboard";
import { DigestChip } from "../DigestChip";
import { ErrorNotice } from "../ErrorNotice";
import { Icon } from "../shell/Icon";
import { FeedbackButtons } from "./FeedbackButtons";
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
  showReasoning = true,
  trace,
  sources,
  onRetry,
  recipeHandle,
  modelId,
}: {
  message: ChatMessage;
  /** Show the model's leading `<think>` reasoning as a collapsed disclosure (T-FEAT1). */
  showReasoning?: boolean;
  trace?: ReactNode;
  /** PR-A: the grounded-answer sources disclosure (read-only RAG); renders nothing
   *  for an ungrounded/unsettled turn. */
  sources?: ReactNode;
  onRetry?: (assistantId: string) => void;
  /** The chat backing handle/model — advisory context on 👍/👎 feedback (PR-4.1). */
  recipeHandle?: string;
  modelId?: string;
}) {
  const { copied, copy } = useCopyToClipboard();
  const inFlight = message.status === "pending" || message.status === "thinking";
  const mod = message.status === "failed" ? " bubble--failed" : inFlight ? " bubble--thinking" : "";
  // Assistant replies may carry a leading <think> reasoning block (raw-committed
  // in the result bytes). Split it from the answer; the answer ALWAYS renders.
  const split =
    message.role === "assistant" && message.text
      ? splitReasoning(message.text)
      : { answer: message.text ?? "", reasoning: undefined };
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
        {/* PR-4.2 (T-STREAM1): an agent chain's LIVE reasoning/acting text streams
            here while in flight — a muted, secondary line so a tool turn's raw
            envelope never poses as the answer. Cleared when the committed answer
            lands (the simple/vision answer streams into `bubble__md` instead). */}
        {message.role === "assistant" && inFlight && message.streamingReasoning ? (
          <div
            className="bubble__stream-reasoning"
            data-testid="bubble-reasoning-stream"
            aria-live="polite"
          >
            {message.streamingReasoning}
          </div>
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
            still match. A leading `<think>` reasoning block is split into a
            collapsed disclosure ABOVE the answer (T-FEAT1) — the answer is never
            hidden; `showReasoning` gates only the disclosure. */}
        {message.role === "assistant" && split.reasoning && showReasoning ? (
          <details className="bubble__reasoning" data-testid="bubble-reasoning">
            <summary>Reasoning</summary>
            <div className="bubble__reasoning-body">{renderMarkdown(split.reasoning)}</div>
          </details>
        ) : null}
        {message.text ? (
          message.role === "assistant" ? (
            split.answer ? (
              // PR-4.2: while the turn is in flight this container renders the LIVE
              // streamed answer (simple/vision); it's an `aria-live` region so a
              // screen reader announces incrementally. On settle the SAME container
              // shows the committed answer (the authority).
              <div
                className="bubble__text bubble__md"
                data-testid="bubble-md"
                data-streaming={inFlight ? "true" : undefined}
                aria-live={inFlight ? "polite" : undefined}
              >
                {renderMarkdown(split.answer)}
              </div>
            ) : null
          ) : (
            <p className="bubble__text">{message.text}</p>
          )
        ) : null}
        {/* The assistant action row: copy the answer + rate it 👍/👎 (PR-4.1).
            Shown only on a settled answer; feedback hides itself on a gateway
            without the seam (don't-fake-gaps). */}
        {message.role === "assistant" && message.status === "done" && split.answer ? (
          <div className="msg-actions" data-testid="msg-actions">
            <button
              type="button"
              className={`msg-action${copied ? " msg-action--on" : ""}`}
              onClick={() => copy(split.answer ?? "")}
              title="Copy answer"
              data-testid="msg-copy"
            >
              <Icon name="copy" size={15} />
              <span>{copied ? "Copied" : "Copy"}</span>
            </button>
            <FeedbackButtons message={message} recipeHandle={recipeHandle} modelId={modelId} />
          </div>
        ) : null}
        {/* PR-A: the grounded-answer sources (read-only RAG) — a compact disclosure
            below the answer; renders nothing when the turn is ungrounded. */}
        {message.role === "assistant" ? sources : null}
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
