import { type ReactNode, useCallback, useEffect, useRef } from "react";
import type { ChatMessage, ChatThread } from "../../lib/chat-thread";
import { EmptyState } from "../EmptyState";
import { MessageBubble } from "./MessageBubble";

/** How close to the bottom (px) still counts as "following" the stream. */
const STICK_THRESHOLD = 96;

/** The scrollable thread; auto-scrolls to the newest message when enabled. */
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
  // Whether the user is following the tail. Starts true; a scroll-up releases it so
  // the stream keeps flowing WITHOUT yanking the viewport back down (the reactivity
  // fix). Scrolling back to the bottom re-arms it.
  const stuckRef = useRef(true);

  const onScroll = useCallback(() => {
    const el = listRef.current;
    if (el) {
      stuckRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_THRESHOLD;
    }
  }, []);

  // Follow the tail only while stuck to the bottom. Keyed on the thread so it fires
  // on every token/message update; a no-op when the user has scrolled up.
  // biome-ignore lint/correctness/useExhaustiveDependencies: follow on any thread change.
  useEffect(() => {
    if (!autoscroll || !stuckRef.current) {
      return;
    }
    const el = listRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight; // jump the container, never scrollIntoView (no page reflow)
    }
  }, [thread, autoscroll]);

  if (thread.messages.length === 0) {
    return <EmptyState title="No messages yet" detail="Ask the runtime something below." />;
  }

  return (
    <div className="chat__list" data-testid="message-list" ref={listRef} onScroll={onScroll}>
      {thread.messages.map((m) => (
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
