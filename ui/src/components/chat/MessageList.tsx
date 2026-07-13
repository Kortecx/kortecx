import { type ReactNode, useCallback, useEffect, useRef } from "react";
import type { ChatMessage, ChatThread } from "../../lib/chat-thread";
import { EmptyState } from "../EmptyState";
import { MessageBubble } from "./MessageBubble";

/** How close to the top (px) still counts as "following" the stream. */
const STICK_THRESHOLD = 96;

/**
 * The scrollable thread. Wave-4: NEWEST-AT-TOP — the freshest turn renders at the
 * TOP and older ones flow down toward the input (the messages are rendered newest-
 * first; the list keeps a natural `flex-direction: column`). Auto-scroll therefore
 * follows the TOP edge (scrollTop → 0), the mirror of the old bottom-follow.
 */
export function MessageList({
  thread,
  autoscroll,
  showReasoning,
  renderTrace,
  renderSources,
  onRetry,
  recipeHandle,
  modelId,
}: {
  thread: ChatThread;
  autoscroll: boolean;
  /** Show the model's `<think>` reasoning disclosure above the answer (T-FEAT1). */
  showReasoning: boolean;
  renderTrace?: (assistantId: string) => ReactNode;
  /** PR-A: the grounded-answer sources for a settled assistant turn (read-only RAG).
   *  Returns nothing for an ungrounded/unsettled turn — never a faked citation. */
  renderSources?: (message: ChatMessage) => ReactNode;
  onRetry?: (assistantId: string) => void;
  /** The chat backing handle/model — advisory context on 👍/👎 feedback (PR-4.1). */
  recipeHandle?: string;
  modelId?: string;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  // Whether the user is following the head (newest-at-top). Starts true; a scroll-DOWN
  // (away from the newest turn) releases it so the stream keeps flowing WITHOUT yanking
  // the viewport back up. Scrolling back to the top re-arms it.
  const stuckRef = useRef(true);

  const onScroll = useCallback(() => {
    const el = listRef.current;
    if (el) {
      stuckRef.current = el.scrollTop <= STICK_THRESHOLD;
    }
  }, []);

  // Follow the head only while stuck to the top. Keyed on the thread so it fires on
  // every token/message update; a no-op when the user has scrolled down to read older
  // turns.
  // biome-ignore lint/correctness/useExhaustiveDependencies: follow on any thread change.
  useEffect(() => {
    if (!autoscroll || !stuckRef.current) {
      return;
    }
    const el = listRef.current;
    if (el) {
      el.scrollTop = 0; // jump the container to the newest (top), never scrollIntoView
    }
  }, [thread, autoscroll]);

  if (thread.messages.length === 0) {
    return <EmptyState title="No messages yet" detail="Ask the runtime something below." />;
  }

  return (
    <div className="chat__list" data-testid="message-list" ref={listRef} onScroll={onScroll}>
      {/* Newest-first: render the reversed thread so the freshest turn sits at the TOP
          (DOM order matches the visual order, keeping scroll coordinates predictable). */}
      {thread.messages
        .slice()
        .reverse()
        .map((m) => (
          <MessageBubble
            key={m.id}
            message={m}
            showReasoning={showReasoning}
            trace={m.role === "assistant" ? renderTrace?.(m.id) : undefined}
            sources={m.role === "assistant" ? renderSources?.(m) : undefined}
            onRetry={m.role === "assistant" ? onRetry : undefined}
            recipeHandle={recipeHandle}
            modelId={modelId}
          />
        ))}
    </div>
  );
}
